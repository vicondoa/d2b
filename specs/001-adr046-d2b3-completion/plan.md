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
therefore does **not** re-derive architecture. It sequences the remaining 531 work items
across waves W2 through W8, resolves the four unknowns that block starting (next-wave scope,
the failed footprint gate, the companion release blocker, and the parity/retirement split),
and defines how each wave passes its gate.

The approach is delivery-shaped, not design-shaped: launch each wave's file-disjoint parallel
groups together, gate every wave on imported validation evidence plus one unanimous ten-role
panel bound to an immutable snapshot, merge through pull requests only after those gates pass,
and cut exactly one release at the end.

Wave 5 now includes an approved production-completion graph in addition to its 146 manifest
items. The graph wires the store, policy, authenticated ComponentSession route, controller
endpoint, watch fan-in, durable effect/adoption ledger, and mutation-audit drainer into one
daemon-owned Zone runtime. It then freezes one candidate and regathers production-boundary
evidence before T219. The earlier backend, watch, and RSS results remain historical inputs;
none substitutes for this final wiring or its exact-candidate evidence.

The C1 contract defect is approved and fully assigned under Constitution 2.2.0. The accepted
provider-system-core member specification is authority for the stable internal handler names,
while the committed unreleased v3 `ZoneHandlerName` enum omitted their status values. T605
adds `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`, serialized as
`system-core-host` and `system-core-user`, and owns its paired snapshots, focused tests,
drift proof, and reference status surface. T595 consumes the variants in the production
emitter and T599 reconciles the remaining consumers; all paired artifacts and evidence land
in the same Wave 5 PR. The plan is now eligible for read-only cross-artifact analysis and, if
no HIGH or CRITICAL finding remains, a unanimous plan panel. It is not eligible for
implementation until both gates pass and T603 completes the receipt-bound progress
reconciliation.

## Technical Context

**Language/Version**: Rust 1.94.1 (pinned via `packages/rust-toolchain.toml`, components
`rustfmt` and `clippy`); Nix for the NixOS module surface

**Primary Dependencies**: redb `=4.1.0` (provisional pin, quarantined in the proof workspace
until the corrected backend lands per D128); ttrpc/protobuf for the resource service; Noise
handshakes for ComponentSession; Cloud Hypervisor and crosvm as runtime backends. No new
toolchain, linter, formatter, or nixpkgs overlay is introduced.

**Storage**: One embedded redb database per Zone, opened by owned fd, with full crash-safe
durability - one fsync per write transaction, no reduced-durability mode. Write queue 256,
group-commit batch 16, read pool 4, concurrent reads 16, read lifetime 250 ms.

**Testing**: Existing closed layer set - nix-unit eval cases, Rust unit and binary integration
tests, rendered-artifact contract tests, policy lints, and flake checks at Layer 1; podman
containers and `runNixOSTest` at Layer 2; hardware, live-host, and cloud tiers manual. No new
top-level shell gate. Every heavy lane runs through the two-slot `xtask heavy-gate` semaphore.

**Target Platform**: `x86_64-linux` NixOS host with KVM, single trusted user. Graphics paths
are x86_64-only by existing platform gate.

**Project Type**: NixOS module framework plus a multi-crate Rust control plane (35 workspace
members today, plus two deliberately excluded standalone workspaces)

**Performance Goals**: Empty-store readiness <=500 ms; p95 local Get and bounded List <=2 ms;
p95 crash-safe single-resource mutation <=10 ms; p95 durable commit to controller handler
start <=5 ms; p95 ready Process commit to launch-attempt start <=20 ms

**Constraints**: Whole-process RSS <=24,576 KiB with **no baseline subtraction** - historical
production fixtures passed at their recorded tips, but the completed production publication
path remains unmeasured until the amended `adr046w5` candidate is frozen; aggregate idle RSS
<=64 MiB; per-component budgets 22 MiB for `Provider/system-core` and 12 MiB for
`Provider/system-minijail`; per-Provider-crate hermetic suite aggregate process-CPU p95 <=3 s

**Scale/Scope**: 531 remaining work items across 53 specs and 7 waves; 27 Provider crates;
19 standard ResourceTypes; hard fixtures at 10,000 resources and 100 concurrent watches

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Assessment | Status |
| --- | --- | --- |
| **I. Daemon-Only Control Plane** | ADR-046 adds per-Zone runtimes as **parent-spawned processes**, not PID1 units, and DELETEs the three per-realm units. Unit count does not grow; the `systemctl list-units` exit criterion is unchanged. Restart remains a continuation event via FR-003. | PASS (see research R5) |
| **II. Broker-Mediated Audited Privilege** | FR-012 keeps every privileged host mutation on the audited broker path; D077 forbids any Provider process importing the broker, enforced by a policy lint. FR-070 adds a daemon-owned resource-mutation audit drainer, not a new service, and requires audit durability before success. `SO_PEERCRED` plus group membership stays the sole local lifecycle authz surface and is never treated as a Resource API subject. | PASS |
| **III. Reasonable Isolation Over Convenience** | FR-009 default-denies cross-Zone reference; FR-014 fails closed on missing identity state rather than reinitializing; FR-066 requires authoritative registrar-derived subjects; FR-069 forbids partial publication; FR-071 isolates a failed Zone without making it ready. virtiofsd zero-capability and per-VM store-farm invariants are untouched. | PASS |
| **IV. Contract-Driven Compatibility** | 3.0 is a deliberate major-version clean break with v3 schemas, versioned artifacts, and fail-closed drift gates (FR-031). Constitution 2.2.0 authorizes this coordinated correction of an approved contract defect before the first d2b 3.0/v3 release. T605 adds the two omitted `ZoneHandlerName` allowed values and moves compiler-derived API snapshots, Rust tests, consumers/emitters, reference status wording, and drift evidence together. The Zone desired-state schema is unchanged. | PASS - C1 resolved in artifacts, implementation pending |
| **V. Test-Layer Discipline** | FR-032 pins coverage to the lowest hermetic layer and forbids a new top-level shell gate; FR-029 routes every heavy lane through the single semaphore; FR-033 retires superseded suites. | PASS |
| **VI. Panel-Gated Multi-Phase Work** | FR-026 requires unanimous ten-role sign-off with zero recommendations per wave. Panels run as 10 read-only subagent lanes on `gpt-5.6-sol` at `xhigh`. Constitution 2.1.0 authorizes pipelined implementation start at 5 of 10 predecessor panels while panel, seal, and merge stay strictly ordered; pipelining is not a current constitution deviation. The W0/W1 delivered-without-panel-or-seal waiver is the one tracked Principle VI and delivery exception. | PASS with one tracked exception |
| **VII. Traceable, Marker-Free Shipped Artifacts** | Wave tags stay in commits and planning artifacts; SC-018 requires the release notes carry zero process markers; FR-019 lands docs with their behavior. ASCII-hyphen rule observed throughout. | PASS |

**Gate result**: **PASS for read-only analysis**. The one tracked Principle VI exception for
W0/W1 remains unchanged. C1 is a Constitution-2.2.0 coordinated defect correction, not a
second constitution deviation. This pass does not authorize implementation: cross-artifact
analysis must return no HIGH or CRITICAL finding, the current plan panel must be unanimous,
and T603 must complete before T589.

**Execution model**: this plan is executed by a coding agent dispatching subagents. Wide
parallel fan-out is a positive obligation, not an optimization - the delivery contract fails
wave entry when a ready, file-disjoint slice is left unlaunched. One write-capable subagent per
parallel group, each in its own worktree; 10 read-only panel lanes on
`gpt-5.6-sol` at `xhigh`. Heavy validation is capped at 2 concurrent lanes by the OFD-locked
semaphore regardless of how many implementation subagents are running. See tasks.md
"Parallel subagent execution model".

**Post-amendment re-check (2026-08-06)**: **PASS for analysis, panel still pending**. The
completion graph introduces no new unit, privileged path, top-level test gate, toolchain,
overlay, compatibility shim, or store-owned policy interpreter. The audit drainer and
controller ledger are Zone-runtime owners inside `d2bd`, and production publication consumes
the existing ComponentSession/ZoneBus boundary. T605 corrects the omitted closed status-enum
values through the approved coordinated path. `spec-coverage.md` binds the local tasks to the
unchanged generated-manifest census.

### C1 correction and version impact

The accepted `docs/specs/providers/ADR-046-provider-system-core.md` member spec already
defines `system_core_host` and `system_core_user` as stable closed handler identities and says
they appear as name/phase/lastReconciledAt entries in `Zone.status.handlers`. The committed
`packages/d2b-contracts/src/v3/zone.rs` enum omitted the corresponding allowed values. T605
therefore adds `ZoneHandlerName::SystemCoreHost` and `ZoneHandlerName::SystemCoreUser`; the
existing kebab-case serialization rule emits `system-core-host` and `system-core-user`.
Readiness consumes exactly one `Zone.status.handlers[]` record for each value.
`ProviderLifecycle` remains a separate aggregate enum value and cannot satisfy either record.

No `apiVersion`, `schemaVersion`, `manifestVersion`, `bundleVersion`, or wire-field version
bump is made. The correction adds no field or operation, changes no desired-state ResourceType
schema, and lands before the first d2b 3.0/v3 release. That decision does not waive paired
artifacts: T605 owns the Rust enum and unit/serialization tests, compiler-derived public and
private API snapshots regenerated only by `make api-surface-pin`, the existing lowest-layer
contract/policy guard, the disjoint reference page
`docs/reference/resource-plane-runtime.md`, and proof that
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

**The manifests are authoritative.** Every work item carries 15 fields, including
`detailedDesign`, `validation`, `destination`, `integration`, `dataMigration`, `currentSource`,
`reuseAction`, `reuseSource`, `dependencyOwner`, and `removalProof`. Those fields are carried
**verbatim** into `tasks.md` and into the implementing change. They are never paraphrased,
condensed, or selectively quoted, because a paraphrase silently drops obligations that the
wave seal will later be asserted against.

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

**Completeness reconciles**: 55 specs, 545 items, 14 `Merged` and 531 `Planned`, splitting
8/6/19/4/32/146/257/73 across W0-W7 with W8 recorded at W7 close. Every item carries a
non-empty `removalProof`, so FR-023's per-path proof obligation is already itemized rather
than needing to be invented.

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
| W2 | 2 | 19 | 2, file-disjoint, zero overlap edges | Ready to launch now |
| W3 | 1 | 4 | 1, strictly serial | Every Provider dossier depends on it |
| W4 | 5 | 32 | 5 parallel | |
| W5 (`adr046w5` delivery address) | 7 | 146 + 17 local completion/resume tasks | 12 manifest groups + the serialized completion graph below | Store exists; production publication and exact-tip evidence remain; analysis, plan panel, and T603 gate resume |
| W6 | 27 | 257 | 5 file-disjoint families | Largest wave; hermetic suites are independent |
| W7 | 5 | 73 | 1 closing group | Destructive cutover |
| W8 | 0 | TBD | friction closure | Terminal; release gate evaluated here |

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
   and every restart it owns one private, sealed, non-`Clone`, one-shot
   `PolicyBootstrapRead`. After immutable Zone/store identity is verified, that capability
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
   the controller endpoint on the exact ZoneBus route. The public daemon bridge may request
   registration but may not construct or pass a subject claim.
4. The admitted ResourceService watch opens through that registered route, replays from the
   durable checkpoint without a replay/live gap, and feeds the registered controller fan-in.
   Before any EffectPort call, the core controller records an outstanding effect in the
   existing per-Zone durable store through the engine-neutral store contract. The core
   controller alone interprets ledger bytes. The key binds Zone, controller generation,
   resource UID, committed revision, operation id, and effect ordinal. Restart adopts or
   idempotently replays pending entries before cleanup. Cleanup completion is a compare against
   the same UID and exact nonzero revision.
5. The Zone runtime owns the mutation-audit drainer. A normal successful mutation response is
   released only after the authoritative sink durably appends the
   operation-id/mutation-ordinal record and the outbox completion is durable. If the mutation
   committed first, the API returns semantic `CommittedPendingAudit` through the existing
   layered `ResourceStatus` composite: `ResourceStatus.phase` is
   `ResourcePhase::Degraded`; `ResourceStatus.outcome.code` is
   `StatusCode("committed-pending-audit")` with retryable, safe remediation and no raw sink
   detail; `ResourceStatus.update.state` is `UpdateState::Blocked`; and
   `ResourceStatus.update.operation_id` is `Some(original_operation_id)`. Existing bounded,
   redacted condition, outcome, and update fields carry only safe same-ID retry/status
   instructions. `ResourceUpdateStatus` gains no phase or status-code member, and no enum
   variant, field, or schema version is added. The result neither reports ordinary success
   nor implies rollback. The Zone is unpublished and degraded until drain recovery. Same-ID
   retry resumes or observes the same mutation and its one stored final result without
   reapplication; a different ID follows normal revision/conflict semantics. Restart
   deduplicates by operation ID plus mutation ordinal and produces one logical audit record.
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

The concrete failures this permits are a committed generation whose process dies after its
effect intent becomes durable but before the effect completes, and a resource mutation that
commits before its audit append can finish. The durable ledger makes the first recoverable;
the operation-bound pending-audit result makes the second observable without lying about
success or rollback. The restart crash-window matrices catch a lost, duplicated, ambiguous,
or stale effect/audit record. The aggregate readiness projection prevents the recovered store
from becoming success-shaped while policy, route, watch, audit, controller, or the exact
system-core Provider ownership is absent.

#### Serial dependencies and file ownership

| Stage | Task(s) | Ownership and concurrency |
| --- | --- | --- |
| Resume reconciliation | T603 | Writes the closed external receipt at `.scratch/autopilot/adr046w5/reconciliation.json`, then routes the sole receipt-bound checkbox transition through `/d2b-spec-edit`. It owns no feature or implementation files directly. |
| Integrator prep | T589 | Its sole direct prerequisite is T603. It remains blocked until the editor progress receipt exists, T073-T218 and T603 are checked, and the reconciliation receipt is current. It lands the shared sealed interfaces and dependency edges first. No implementation slice branches before this commit. |
| File-disjoint implementation | T590-T594, T605 | Six slices start together from T589. T590-T594 own policy, D106, store/audit persistence, authenticated routing, and controller ledger files. Read-only inspection found no competing feature-task owner for the existing `packages/d2b-contract-tests/tests/policy_contracts.rs`, and `docs/reference/resource-plane-runtime.md` exists and canonically documents the Zone runtime. T605 alone owns those two files, `packages/d2b-contracts/src/v3/zone.rs`, and compiler-regenerated public/private outputs under `tests/golden/api-surface/`; snapshots are regenerated only by `make api-surface-pin`. It treats `docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json` and `packages/xtask/src/zone_schema.rs` as read-only proof inputs: generator output must remain byte-identical. No slice edits `d2bd/src/resource_runtime.rs` or `d2bd/src/lib.rs`. |
| Serial daemon composition | T595 | Sole writer for `d2bd/src/resource_runtime.rs`, `d2bd/src/lib.rs`, and `d2bd/Cargo.toml`; begins only after all six slices converge. |
| File-disjoint acceptance and docs | T596-T599, T604 | T604 owns new `packages/d2b-contract-tests/tests/resource_operator_activation.rs`, `packages/d2bd/tests/resource_operator_activation.rs`, and `tests/host-integration/resource-operator-activation.nix`; the other tasks retain their named files. All five tasks may proceed together after T595 and share no file. |
| Frozen-candidate evidence | T600-T601 | Read-only evidence lanes run against one frozen commit. They write delivery evidence only, not repository files, and may run together subject to the heavy-gate limit. |
| Mechanical convergence | T602 | Verifies dependency closure and exact-candidate bindings. T219 is blocked until it passes. |

The implementation dependency chain is exactly
`T589 -> {T590,T591,T592,T593,T594,T605} -> T595`.

T603 is the sole direct prerequisite of T589, but it never treats code presence as completion
and never edits a feature artifact itself. It writes one machine-readable reconciliation
receipt outside Git at `.scratch/autopilot/adr046w5/reconciliation.json`. The receipt has this
closed shape; every object rejects unknown fields:

- top level: `schema_version` exactly `1`; `wave` exactly `adr046w5`; `feature_dir` exactly
  `/home/paydro/projects/d2b-w5wave/specs/001-adr046-d2b3-completion`; `feature_files`;
  `feature_snapshot_sha256` as lowercase 64-hex; `git_tip` as the full lowercase 40-hex
  commit ID; nonempty `branch` and local delivery `candidate` strings; `analysis`;
  `plan_panel`; and `items`;
- `analysis`: `result`, whose closed values are `pass` and `fail`; the same
  `feature_snapshot_sha256` and `git_tip` as the top level; and one nonempty local
  `receipt_locator` or session locator, with no transcript;
- `plan_panel`: `round` matching `^adr046w5-r[1-9][0-9]*$`;
  `reviewed_feature_snapshot_sha256`; `reviewed_git_tip`; `unanimous`; and
  `record_locators`, an object with exactly one nonempty locator for each of `software`,
  `test`, `nixos`, `networking`, `security`, `rust`, `product`, `docs`, `observability`, and
  `kernel`, and no verdict text; and
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
of the concatenated stream.

Receipt creation fails on an unknown field, unknown status or evidence kind, task-ID
duplicate, missing or extra task, identity mismatch, stale feature hash, stale `git_tip`,
branch or candidate mismatch, analysis identity mismatch, panel reviewed-tip/snapshot
mismatch, malformed round address, non-unanimous or incomplete panel records when progress is
requested, or a locator that is absent or not local delivery state. The receipt contains only
addresses, classifications, and outcomes. It contains no diff, transcript, command or
validation output, secret, credential, Nix store path, or raw sink detail.

The parent directory is mode `0700`. Receipt creation uses a same-directory mode-`0600`
regular temporary file owned by the invoking uid, writes complete canonical UTF-8 JSON,
flushes and fsyncs it, atomically renames it over `reconciliation.json`, and fsyncs the
directory. A symlink, wrong owner, broader mode, cross-filesystem replacement, partial write,
or failed fsync is a hard failure. Readers re-run every structural and identity check; file
existence alone is not a receipt.

If any row is `open`, T603 stays unchecked and no checkbox changes occur. If and only if all
146 rows are `satisfied`, `analysis.result` is `pass` with no unresolved HIGH or CRITICAL
finding, and the current panel is unanimous on the same tip and snapshot, T603 prepares one
explicit `/d2b-spec-edit` progress batch citing the exact receipt path and SHA-256. Before
editing, the feature editor recomputes the listed-file snapshot, Git tip, branch, candidate,
analysis identity, panel identity, row set, and receipt SHA-256. The batch may change only
`tasks.md`, and only by changing the checkbox token for T073-T218 and T603 from unchecked to
checked. It makes no prose or other feature change.

The editor's machine-readable progress receipt is persisted outside Git at
`.scratch/autopilot/adr046w5/progress-editor-receipt.json`. It binds the reconciliation
receipt SHA-256, pre-edit feature snapshot, post-edit feature snapshot, unchanged Git tip,
and the exact changed task-ID set T073-T218 plus T603. T589 refuses unless that editor receipt
passes the same ownership, mode, atomicity, set, and identity checks and `tasks.md` shows all
147 checkboxes checked. The reconciliation hash is compared to the editor receipt's pre-edit
snapshot, while the current feature tree is compared to its post-edit snapshot; the only
permitted difference between those snapshots is the 147 checkbox tokens. This prevents the
authorized transition from making its own pre-edit receipt appear stale while still rejecting
any later prose or checkbox drift. This editor-mediated batch is the sole authorized progress
transition; an integrator or autopilot lane never edits those checkboxes directly.

T589 may touch files later owned by a serialized successor because its purpose is to
establish the contracts those successors implement. No two parallel tasks own the same file.
Cargo workspace membership, generated spec manifests, flake outputs, shared changelog, and
feature artifacts remain integrator-only.

#### Wave 5 validation and evidence

The implementation tasks run focused hermetic tests while writing their files. Before T595,
T605 runs the `d2b-contracts` unit/serialization cases, the existing
`policy_contracts.rs` Layer-1 policy surface, `make api-surface-pin` followed by the API
snapshot check, and the existing schema drift gate. Its schema proof compares
`docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json` with the T589 base and requires no
byte change after generator execution; the schema is never hand-edited. After T595, T596-T599
and T604 fan out together. T596-T598 add production-boundary tests that enter through the
daemon/session boundary and the registered ZoneBus route, T599 reconciles reference behavior,
and T604 adds the missing operator boundary. T604 first verifies the emitted Nix resource
bundle in
`packages/d2b-contract-tests/tests/resource_operator_activation.rs`. Its lowest feasible
production-boundary leg is the Type-3
`packages/d2bd/tests/resource_operator_activation.rs`, which consumes those exact generation
bytes through the daemon activation/reload entry and production store/controller path without
calling ResourceService directly. Run those legs through
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` and `make test-rust`. Because a real
owned host effect requires systemd, broker mutation, and a booted NixOS system, the canonical
Type-10 destination is `tests/host-integration/resource-operator-activation.nix`, run only by
the public heavy-gated `make test-host-integration` target. It declares the representative
Guest, Volume, Network, and Device through Nix, activates through the production daemon,
observes durable creation and reconciliation, removes the Guest, reactivates, and observes
dependency-safe cleanup while the unrelated resources remain intact. A direct
ResourceService or `WatchService` call, `ProductionWatchHarness`, fixed subject, fake
endpoint, or manually set readiness field may remain useful unit coverage but is explicitly
ineligible as T219 evidence.

After code and documentation freeze, T600 and T601 import records bound to the same candidate:

- authenticated same-Zone request/watch and cross-Zone/self-named-subject denial;
- restart at each generation-commit/effect-ledger/dispatch/completion window, plus stale,
  zero, wrong-UID, and ambiguous cleanup negatives;
- mutation audit append, outbox completion, crash replay, and logical deduplication;
- committed-before-audit failure returning `CommittedPendingAudit`, same-ID no-reapply and
  eventual-final-result behavior, different-ID revision/conflict behavior, and degraded
  status observability;
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
  security-key, Resource API, and error reference pages; and
- T604's exact-candidate operator activation-to-effect-and-cleanup result, including the
  emitted bundle identity, production daemon entry point, per-resource effect/readiness or
  precise actionable refusal, removed-resource cleanup, and unchanged unrelated resources.

T602's done condition is mechanical: T603, every T589-T599 task, T604, and T605 are complete;
`tasks.md` shows T073-T218 and T603 checked; the external reconciliation and editor progress
receipts pass every identity and closed-set check; every T073-T218 receipt row is satisfied;
both evidence manifests name the candidate commit produced after T599, T604, and T605; every
required evidence kind is present exactly once; both manifests reference T604's
exact-candidate activation result and T605's contract/drift result; no record names a
direct/fake boundary; same-ID audit retry never reapplies a mutation; the pending state uses
the exact existing `ResourceStatus` composite; the system-core registration and both required
handler records are healthy and unique; current removal proofs are true; the RSS value is at
or below 24,576 KiB; and the candidate tree is unchanged after evidence capture.

This amendment changes the reviewed Wave 5 plan boundary. Cross-artifact analysis and a new
unanimous plan panel are captured by T603 before T589; implementation does not resume on the
strength of this edit alone. The current plan panel uses `/d2b-panel-round plan` with
qualified wave `adr046w5` and a round address of the form `adr046w5-r<n>`. C1 is approved and
fully assigned, so the next eligible action is read-only analysis. Only after analysis has no
HIGH or CRITICAL finding, a unanimous current plan receipt exists, and the editor progress
receipt proves the checkbox transition may `/d2b-autopilot --resume` continue from the
`adr046w5` checkpoint. T603 still reconciles exactly T073-T218; it records T605 only as
future work after resume and does not add a 147th receipt row or a 148th checkbox transition.

### Recorded drift

`ADR-046-validation-and-delivery` §3.2 lists `packages/d2b-process/` and
`packages/d2b-provider-supervisor/` under W2. No W2 work item targets either path; the owning
item `ADR046-process-001` is W4. Per "existing code is canon" the machine-readable graph wins.
This plan follows the graph. Correcting the prose is a specification amendment that re-opens
that spec's evidence, so it is raised to the integrator rather than fixed mid-wave.

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
unmeasured until T600 runs against the frozen amended candidate.

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
evidence to be regathered rather than carried forward, so the current `adr046w5` delivery must
re-snapshot, re-import, and re-request its panel before it may seal. No wave has sealed under
the superseded text, so no merged wave needs re-panelling.

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
validation obligations are met; T600 must measure the amended candidate and T602 rejects either
2026-08-03 artifact as current evidence.

The backend commit predates the amendment base, so it is outside this focused Gate 0 review and
is not reverted or re-adjudicated here.

## Complexity Tracking

### Constitution Principle VI exception

| Exception | Why Needed | Simpler Alternative Rejected Because |
| --- | --- | --- |
| W0 and W1 delivered without the panel and seal that Principle VI and the delivery contract's exit criteria require (FR-034) | Both waves are already merged. Their binding panel would have to run against an immutable snapshot that no longer exists in a single canonical form - delivery state holds ten competing W0 candidates, one panel-request, zero receipts, and zero seals, and W1 has no delivery state at all. The condition W2 close actually tests, every prior work item recorded `Merged`, is satisfied; per FR-057 that is an exit condition, not an entry one. | Retroactively panelling and sealing both waves was rejected: it would attest to a reconstructed snapshot, which is weaker evidence than an honest waiver and sets a worse precedent than admitting the gap. Renumbering so the first sealed wave becomes W0 was rejected: it invalidates the committed implementation graph, the work-item manifest, and 445 historical commits carrying legacy `ADR046-W0` tags for no verification gain. FR-035 confines the waiver to a one-time exception, and SC-021 forces the waived foundation to become production-reachable, which re-tests it under real load. |

This is the only accepted constitution deviation and delivery exception. C1 is not a second
exception: Constitution 2.2.0 authorizes its coordinated contract correction, which is fully
assigned to T605. No implementation is claimed, and implementation remains gated on
cross-artifact analysis, unanimous plan signoff, and T603.

### Program-local safety and delivery risks

These rows are not Constitution Principle VI deviations.

| Risk | Why Tracked | Guard and Rejected Alternative |
| --- | --- | --- |
| FR-043 (recovery-point attestation) is tracked program-local, outside the work-item manifest, so the W7 seal does not enforce it | FR-043 is locally added and **stricter** than `ADR-046-reset-and-cutover`, which permits proceeding past the rollback boundary without attestation. Creating a manifest work item would require amending that member spec, which re-opens its validation and panel evidence and re-triggers Gate 0. | Amending the spec was considered and rejected for cost. The accepted consequence is explicit: **a green W7 seal is not evidence that FR-043 shipped.** T580 and the W7 merge review are the only enforcement. This is the highest-consequence program-local safety gap because FR-043 is the primary control for the accepted daily-driver validation risk; if it slips, it slips silently. |
| Pipelined dispatch can create successor rework when a predecessor panel finding invalidates in-flight work | Constitution 2.1.0 expressly authorizes implementation of wave N+1 to begin at 5 of 10 wave-N panel returns plus green integration. It is therefore current policy, not a constitution deviation. | Unanimity, roster, seal ordering, and merge ordering remain unchanged. The successor rebases onto the merged predecessor before its own panel, so no panel reviews a tree built on unreviewed contracts. Strict serialization was rejected because it adds idle time without strengthening the exit gate. FR-050 forbids citing rework as grounds to shorten a panel. |
