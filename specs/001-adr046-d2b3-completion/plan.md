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
groups together, converge every repository change before exact-candidate evidence, gate every
wave on imported validation evidence plus one unanimous ten-role panel bound to that immutable
snapshot, merge the attested tree through a pull request without a post-attestation content
change, and cut exactly one release at the end.

Wave 5 now includes an approved production-completion graph in addition to its 146 manifest
items. The graph wires the store, policy, authenticated ComponentSession route, controller
endpoint, watch fan-in, durable effect/adoption ledger, and mutation-audit drainer into one
daemon-owned Zone runtime. T220 then converges every slice and integrator-owned generated
artifact before freezing final candidate F; T600-T602 regather and check production-boundary
evidence against F before T219 runs the wave's one binding panel, seal, and merge. The earlier
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
the checkbox transition. T589 remains blocked until that post-validator gate and the
receipt-bound progress reconciliation both pass.

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
| **II. Broker-Mediated Audited Privilege** | FR-012 keeps every privileged host mutation on the audited broker path; D077 forbids any Provider process importing the broker, enforced by a policy lint. FR-070 adds a daemon-owned resource-mutation audit drainer, not a new service, and requires audit durability before success. `SO_PEERCRED` plus group membership stays the sole local lifecycle authz surface and is never treated as a Resource API subject. | PASS |
| **III. Reasonable Isolation Over Convenience** | FR-009 default-denies cross-Zone reference; FR-014 fails closed on missing identity state rather than reinitializing; FR-066 requires authoritative registrar-derived subjects; FR-069 forbids partial publication; FR-071 isolates a failed Zone without making it ready. virtiofsd zero-capability and per-VM store-farm invariants are untouched. | PASS |
| **IV. Contract-Driven Compatibility** | 3.0 is a deliberate major-version clean break with v3 schemas, versioned artifacts, and fail-closed drift gates (FR-031). Constitution 2.2.0 authorizes this coordinated correction of an approved contract defect before the first d2b 3.0/v3 release. T605 owns the two omitted `ZoneHandlerName` values plus its normative, test, API-snapshot, guard, and reference artifacts; T595 owns emission, T599 owns downstream consumers, and T220 owns generated-manifest reconciliation and the full drift gate. Those stages land together in one Wave 5 PR. The Zone desired-state schema is unchanged. | PASS - C1 resolved in artifacts, implementation pending |
| **V. Test-Layer Discipline** | FR-032 pins coverage to the lowest hermetic layer and forbids a new top-level shell gate; FR-029 routes every heavy lane through the single semaphore; FR-033 retires superseded suites. | PASS |
| **VI. Panel-Gated Multi-Phase Work** | Every immutable candidate receives at most one unanimous binding ten-role panel with zero recommendations. Panels run as 10 read-only subagent lanes on `gpt-5.6-sol` at `xhigh`. A nonunanimous candidate remains immutable and failed; scoped fixes, delta/full-context follow-up panels iterated to unanimous closure, convergence, and validation produce a distinct successor with its own one request. Constitution 2.1.0 authorizes pipelined implementation start at 5 of 10 predecessor reviews while panel, seal, and merge stay strictly ordered. The external ADR/tooling still says once per wave, so versioned feature-local prerequisite `adr046-candidate-recovery-prerequisite/v1` remains binding. T008, T030, and T037 are now historical entry attestations because dependent implementation already exists; none may be checked from a current rerun. T029, T036, and T071 fail closed unless exactly one contemporaneous historical or candidate-bound remedial disposition exists. W0/W1 delivered without panel/seal remains the existing tracked exception. | PASS for the design; historical drift is explicit and wave close is fail-closed |
| **VII. Traceable, Marker-Free Shipped Artifacts** | Wave tags stay in commits and planning artifacts; SC-018 requires the release notes carry zero process markers; FR-019 lands docs with their behavior. ASCII-hyphen rule observed throughout. | PASS |

**Gate result**: **PASS for pre-T603 read-only analysis**. The one tracked Principle VI
exception for W0/W1 remains unchanged. C1 is a Constitution-2.2.0 coordinated defect
correction, not a second constitution deviation. A no-HIGH/CRITICAL analysis and unanimous
plan panel at A/P0 authorize only T603's validator paths. After validator-only V becomes B,
analysis and the plan panel rerun at B/P; only those post-validator receipts and T603's
receipt/editor transition authorize T589.

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
| W2 | 2 | 19 | 2, file-disjoint, zero overlap edges | 19 manifest `Merged`; T008 historical attestation or F2 remedial requalification required before close |
| W3 | 1 | 4 | 1, strictly serial | Every Provider dossier depends on it |
| W4 | 5 | 31 | 6 parallel | 31 manifest `Merged`; T037 historical attestation or F4 remedial requalification required before close |
| W5 (`adr046w5` delivery address) | 7 | 146 + 17 local completion/resume tasks | 12 manifest groups + the serialized completion graph below | Store exists; production publication and exact-tip evidence remain; pre-T603 A/P0 gates and post-T603 B/P gates precede resume |
| W6 | 27 | 258 | 5 file-disjoint families | Includes deferred `ADR046-process-002`; largest wave; hermetic suites are independent |
| W7 | 5 | 73 | 5 parallel | Destructive cutover |
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
   Peer-pidfd acquisition uses only a pinned, reviewed safe dependency implementation. It
   checks exact kernel-returned `optlen`, returns only `OwnedFd`, closes any fd returned on a
   short/malformed or later failure, and reports a typed error without assert, panic, or leak.
   Workspace `unsafe_code = "forbid"` rules out a local syscall fallback in
   `d2b-session-unix`; absence of a qualifying dependency blocks the slice.
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
7. Installed-host migration publishes one durable, replayable handoff binding the previous and
   target system, daemon, bundle-pointer, and complete bundle-set generations. `d2bd` carries
   only an opaque handoff identity. T592's typed privileged broker operation exclusively owns
   system-profile and pointer publication, the exact existing `d2bd.service` restart, and
   rollback; every phase is immutable-audited, and daemon/Nix direct mutation is denied.
   Pointer publication is file- and directory-durable before the typed restart phase. Crash
   replays publication, restart, or readiness acknowledgement idempotently;
   restart/readiness failure durably rolls the complete matching generation back. An
   identical switch or replayed acknowledgement causes no second ingestion or effect.
   Runtime version refusal is identifier-free, has fixed redacted `Debug`, and carries only
   closed action `rebuild-host-generation`. Reference documentation maps that action to a
   pasteable command which reads the complete flake output reference from the root-owned
   stable reference file; runtime output contains neither the command nor the reference.

The concrete failures this permits are a committed generation whose process dies after its
effect intent becomes durable but before the effect completes, and an audit segment export
that fails after the mutation and its immutable authoritative journal row commit together.
The durable ledger makes the first recoverable; the operation-bound pending-audit result makes
the second observable without lying about success or rollback. The restart crash-window
matrices catch a lost, duplicated, ambiguous, or stale effect/export record, while the
transactional journal prevents an unaudited committed privilege change. The aggregate
readiness projection prevents the recovered store from becoming success-shaped while policy,
route, watch, audit export, controller, or the exact system-core Provider ownership is absent.

#### Serial dependencies and file ownership

| Stage | Task(s) | Ownership and concurrency |
| --- | --- | --- |
| Resume reconciliation | T603 | Pre-T603 analysis and plan panel at A/P0 authorize only `packages/xtask/src/delivery/{mod.rs,resume.rs}`. T603 lands dedicated validator commit V with sole parent A and no other repository change, freezes B exactly at V and P byte-identical to P0, reruns analysis and the plan panel at B/P, then and only then writes the immutable authorization receipt at `.scratch/autopilot/adr046w5/reconciliation.json` and routes the sole receipt-bound checkbox transition through `/d2b-spec-edit`. It writes no feature prose; the Wave 5 integrator alone owns dedicated checkbox commit C. |
| Integrator prep | T589 | Its sole direct prerequisite is T603. Candidate-recovery v1 must be accepted on T589's actual base, but T008 is a separate historical W2 attestation and is not retroactively completed by T589. T589 remains blocked until the finalized editor progress receipt exists, T073-T218 and T603 are checked, and HEAD is the clean dedicated checkbox commit. It lands the shared sealed capability, transactional audit-journal hook, mutation-response wire, and typed `InspectOperation` store/API/protobuf contract. It does not edit broker wire, StoreSync DTOs, their callers, broker protocol metadata, schemas, fingerprints, or snapshots. It also owns the `adr046w5` closed-evidence profile and candidate-scoped strict state in `packages/xtask/src/delivery/{evidence,panel,seal,eligibility,history_proof,storage}.rs`: one fd-anchored, file-and-directory-durable, no-replace active reservation per program/wave; fixed redacted state/errors; fd-relative durable orphan cleanup; one request per immutable candidate; synchronized cross-candidate exclusion; the point-specific zero/zero-or-one/exactly-one crash oracle; durable failed closure before active-slot release; and successor admission only with complete predecessor/program-wave/recommendations/successor-commit-tree/convergence/validation identities. Crash injection covers panel-request publication, closure, release, and successor admission. Post-request moves and retries fail at panel, seal, and eligibility. No implementation slice branches before this commit. |
| Serialized implementation | T590-T594, T605 | T590, T591, and T594 start together from T589. T592 starts only after T591 because both own `transaction.rs`; T593 starts after T592 because both own `packages/Cargo.lock`; T605 starts after T593 and regenerates the shared API snapshots. T592 is the sole atomic broker-wire owner: it changes both StoreSync DTOs with every producer/consumer, the typed broker drain, and the typed broker-mediated host-generation publish/restart/rollback op; bumps `PROTOCOL_VERSION` 4 to 5; updates v4/v5 compatibility tests; and regenerates every affected schema, capability fingerprint, reference table, and serialization snapshot in that same commit. T592 also owns `d2b-audit`; `packages/d2b-contracts/src/generation_bundle.rs` as authoritative `ZoneBundle`; retirement of the duplicate full envelope; and a nonempty structural/API poison guard rejecting any second envelope or alias, version authority, hash implementation/entry point, or re-export. T593 owns the session adapter, descriptor, socket, bus Unix transport, session seam and tests, all consuming one accepted-socket evidence object through a reviewed safe exact-optlen dependency returning `OwnedFd`; a local unsafe/syscall fallback is forbidden. T605 owns its Zone enum, governing specs, paired reference, and final API snapshots; generated manifests remain T220-only. T592's serialized commit may edit `d2bd/src/lib.rs`; after it lands, ownership transfers to T595. No other slice edits `d2bd/src/resource_runtime.rs` or `d2bd/src/lib.rs`. |
| Serial daemon composition | T595 | Sole writer after T592 for `d2bd/src/resource_runtime.rs`, `d2bd/src/lib.rs`, `d2bd/Cargo.toml`, and `nixos-modules/{bundle-zones,host-daemon,options-site}.nix`; begins only after T590, T592, T594, and T605 converge. It owns startup ingestion, the daemon/client `InspectOperation` path, exact 4/2 carrier consumption, and the unprivileged handoff state machine. All profile/pointer publication, exact existing-service restart, and rollback effects go through T592's typed audited broker op; direct daemon/Nix mutation is a test failure. It emits the required root-owned stable host rebuild reference, keeps its value out of diagnostics, and adds no unit. Runtime version refusal has fixed redacted `Debug` and only closed action `rebuild-host-generation`; the exact placeholder-free command is documentation-only. |
| File-disjoint acceptance and docs | T596-T599, T604 | T599 owns the ResourceService-backed operation-inspection CLI, retained `--deadline`/`--no-deadline` controls, safe static human guidance, version-2 DTO/schema and contract tests, the coordinated `ADR-046-cli-and-operations` version amendment and migration guidance, and downstream status reconciliation with T595/T605. T604 owns new `packages/d2b-contract-tests/tests/resource_operator_activation.rs`, `packages/d2bd/tests/resource_operator_activation.rs`, `tests/host-integration/resource-operator-activation.nix`, and only the host-integration discovery/build recipe in `Makefile`. Its evidence must enumerate and build `vmChecks.x86_64-linux.resource-operator-activation`; skip or empty discovery is ineligible. The other tasks retain their named files. All five tasks may proceed together after T595 and share no file. |
| Integrator convergence and freeze | T220 | Merges every slice; reconciles generated manifests; verifies coordinated normative/reference/test/schema/changelog treatment, including the authoritative single-owner 4/2 bundle contract/compiler/schema, poison guard, canonical digest, old/mixed/future refusals, replayable installed-host migration/rollback, closed runtime action, and command-only docs; folds fragments; rebases after W4; and records the panel base. It runs the closed-evidence profile plus the point-specific reservation oracle, durable orphan cleanup, concurrent reservation, panel-request/closure/release/successor crash transitions, same-candidate retry, active-alternate-candidate, malformed/stale/cross-candidate/cross-wave recovery-evidence matrix, and post-request movement tests at panel, seal, and eligibility. It then runs integration, CI, and full drift, opens or updates the PR, and freezes F. Any later content/history change invalidates F and restarts T220 plus T600-T602. |
| Frozen-candidate evidence | T600-T601 | Read-only evidence lanes run against F. They write delivery evidence only, not repository files, and emit the exact closed validation identifiers assigned below. They may run together subject to the heavy-gate limit. |
| Mechanical evidence convergence | T602 | Verifies dependency closure, resume identities, clean F, and the exact evidence-identifier multiset. T219 is blocked until it passes. |
| Single binding close and merge | T219 | Runs pre-panel checks, then F's one binding panel, seal, and merge. F stays immutable. Nonunanimity fails F, routes scoped fixes back through T220 and T600-T602, and requires a delta/full-context follow-up panel before a distinct successor's one request. External policy/tooling refusal escalates; it never waives findings. |

The implementation and close dependency chain is exactly:

```text
W2 implementation exists -> {T008 exact historical attestation OR F2 remedial requalification} -> T029 W2 close
W2 close -> W3 implementation exists -> {T030 exact historical attestation OR proposed-F3 remedial requalification} -> T035 freeze -> T036 W3 close
W3 close -> W4 implementation exists -> {T037 exact historical attestation OR F4 remedial requalification} -> T071 W4 close
W4 close -> pre-T603 analysis + plan panel at A/P0
pre-T603 analysis + plan panel at A/P0 -> T603 validator commit V
V = B -> post-T603 analysis + plan panel at B/P -> receipt/editor transition C
C -> T589 -> {T590,T591,T594}
T591 -> T592
T592 -> T593 -> T605
{T590,T592,T594,T605} -> T595
T595 -> {T596,T597,T598,T599,T604} -> T220 -> freeze F
F -> {T600,T601} -> T602 -> T219
```

T603 is the sole direct prerequisite of T589 and never treats code presence as completion.
Its gate is deliberately two-pass because the validator cannot attest to a base that predates
its own implementation:

1. **Pre-validator authorization.** Freeze clean commit A and the exact 28-file feature
   snapshot P0. Run `/speckit-analyze` against the feature artifacts at A/P0. Only a receipt
   with no unresolved HIGH or CRITICAL finding may proceed to the unanimous
   `/d2b-panel-round plan` review bound to A/P0. These receipts authorize only T603's two
   source paths; they do not authorize a reconciliation receipt, checkbox edit, T589, or any
   other Wave 5 implementation.
2. **Validator implementation.** T603 changes exactly
   `packages/xtask/src/delivery/{mod.rs,resume.rs}` and its tests within those files. It
   writes no feature artifact, generated output, source elsewhere, or tracked evidence. The
   integrator lands one dedicated validator commit V whose sole parent is A and whose diff is
   limited to those paths.
3. **Post-validator authorization.** Freeze resume base B exactly at V and recompute feature
   snapshot P. P MUST be byte-identical to P0 because T603 has no feature-file ownership.
   Rerun `/speckit-analyze` over `A..B` plus the complete feature artifacts, then rerun
   `/d2b-panel-round plan`; both new receipts MUST bind B and P, and the panel request MUST
   expose the validator delta. A finding or any subsequent validator-code change abandons B.
   A source-only fix creates a new V/B and reruns both post-validator gates. A finding that
   requires a feature-artifact change returns to a fresh `/d2b-spec-edit` batch, establishes a
   new A/P0, and reruns the entire pre-validator and post-validator sequence. Neither receipts
   from the old A/P0 nor receipts from the abandoned B may be reused.
4. **Receipt/editor transition.** Only the passing post-validator B/P receipts permit T603 to
   create the immutable resume authorization, evaluate the 146 rows, and route the sole
   checkbox change through `/d2b-spec-edit`.

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

T589 starts only with HEAD exactly C, a clean complete worktree, the finalized progress
receipt, and all 147 checkboxes checked. Later implementation commits do not stale the
authorization: T602 validates the immutable receipt against B/P, the finalized transition
against C/Q, requires C to be the exact child of B and an ancestor of final candidate F, and
validates T600/T601 separately against F and F's tree. This ancestry-and-snapshot chain is the
sole authorized progress transition; neither receipt is reinterpreted as evidence for the
final candidate.

T589 may touch files later owned by a serialized successor because its purpose is to
establish the contracts those successors implement. No two parallel tasks own the same file.
Cargo workspace membership, generated spec manifests, flake outputs, shared changelog, and
feature artifacts remain integrator-only.

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
operator boundary. T220 then reconciles the integrator-owned generated spec manifests and
runs the full drift gate before F is frozen. T604 first verifies the emitted Nix resource
bundle in
`packages/d2b-contract-tests/tests/resource_operator_activation.rs`. Its lowest feasible
production-boundary leg is the Type-3
`packages/d2bd/tests/resource_operator_activation.rs`, which consumes those exact generation
bytes through the daemon startup/change-ingestion entry and production store/controller path
without calling ResourceService directly. Run those legs through
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` and `make test-rust`. Because a real
owned host effect requires systemd, broker mutation, and a booted NixOS system, the canonical
Type-10 destination is `tests/host-integration/resource-operator-activation.nix`, run only by
the public heavy-gated `make test-host-integration` target. It declares the representative
Guest, Volume, Network, and Device through Nix, switches through the public NixOS activation
path without a manual daemon restart or private reload, and requires every one of those four
supported resources to reach its real owned effect and readiness. The target must discover a
nonempty `vmChecks` set, enumerate and successfully build
`vmChecks.x86_64-linux.resource-operator-activation`, and report no skip; skipped or empty
output is ineligible evidence. Typed refusal cases are separate negative tests and cannot
satisfy the positive story. The test removes the Guest, switches again, and observes
dependency-safe cleanup while the unrelated resources remain ready and intact. A direct
ResourceService or `WatchService` call, `ProductionWatchHarness`, fixed subject, fake
endpoint, or manually set readiness field may remain useful unit coverage but is explicitly
ineligible as T219 evidence.

T589 implements one hermetic `adr046w5` closed-evidence profile in the delivery validator
before any final candidate can be frozen. The profile compares the imported record multiset
to the following table byte-for-byte and is invoked at panel-request, seal, and
merge-eligibility, not only by T602 prose. Its table-driven negative suite must independently
reject a missing row, an extra row, a duplicate pair, an unknown identifier, a right
identifier in the wrong lane, and one record conflating two required rows. T220 reruns that
suite before freezing F. After T220 completes every repository change and freezes F, T600 and
T601 import generic `EvidenceRecord` objects bound to F. T602 invokes the same validator and
adds the receipt, ancestry, and clean-tree checks; it does not implement or substitute a
second validator.

| Closed `validation` identifier | Owner | Delivery lane | Required content |
| --- | --- | --- | --- |
| `production-session-watch` | T600 | `github-ci` | Same-Zone request/watch through the production session/router plus cross-Zone and self-named subject denial; one accepted-socket evidence object shared by adapter, descriptor, bus Unix transport, and session seam; reviewed safe dependency `SO_PEERPIDFD` implementation with exact-optlen validation, CLOEXEC `OwnedFd`, and returned-fd failure closure; injected short-result fd-count, unsupported/malformed/missing-CLOEXEC/leak/dead/numeric-only/reuse/credential/generation/cgroup/ambiguity refusals; workspace unsafe-forbid and no local syscall/raw-fd fallback; no caller-supplied verifier/credential constructor or re-export; private registrar issuance; and a fresh restart peer pidfd |
| `effect-replay-cleanup` | T600 | `github-ci` | Every generation/effect-ledger crash window, replay/adoption, and stale/zero/wrong-UID/ambiguous cleanup refusal |
| `audit-drain-replay` | T600 | `github-ci` | Transactional authoritative rows; fd-anchored file/directory-durable export and restart replay; digest-plus-ordinal deduplication; durable operation inspection; replay-binding denials; fixed-digest/record limits; valid-present/absent/malformed trace behavior with typed correlation and no fabrication/relabel; typed `StoreSyncRequest`/`StoreSyncResponse` producers, consumers, schema and snapshots; fixed redacted `Debug` for every migrated producer, both StoreSync wire DTOs, broker-drain DTO, SegmentWriter/sink/export/directory/opaque-handle owner; post-export journal retention and prune health; and raw identifier/trace/path/handle-canary absence |
| `system-core-handler-contract` | T600 | `github-ci` | T605 enum/list/API/reference/schema proof, T595 emission, T599 consumer agreement, exact live handler readiness, non-substitution, and multi-Zone isolation |
| `operator-nix-activation-cleanup` | T600 | `local-host` | T604 exact 4/2 top-level audit carrier, audit-only generation identity change, empty ZoneSpec/no emitted Zone resource, 3/1/mixed/future 5/2, 4/3, and 5/3 plus missing/misplaced refusals; durable replayable installed-host handoff from 3/1 to 4/2; typed broker-only profile/pointer publication, exact existing-service restart, rollback, and immutable audit rows; direct daemon/Nix mutation refusals; pointer/restart/readiness crash and restart-failure rollback matrix with matching generations and no duplicate effect; closed identifier-free `rebuild-host-generation` runtime action, placeholder-free stable-reference command in docs only, command execution in host recovery coverage, and no reference value in diagnostics; initial, identical, declaration, and removal public switches; four representative owned effects/readiness; dependency-safe cleanup; ready unrelated resources; and nonempty host output enumerating and building `vmChecks.x86_64-linux.resource-operator-activation` without skip |
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
  emitted bundle identity, public switch-triggered production daemon entry point, owned
  effect/readiness for every representative Guest, Volume, Network, and Device, cleanup of
  the removed resource, and unchanged ready unrelated resources.

T602's done condition is mechanical: T603, every T589-T599 task, T604, T605, and T220 are complete;
`tasks.md` shows T073-T218 and T603 checked; the immutable reconciliation receipt validates
against resume base B and pre-edit snapshot P; the progress receipt validates dedicated
checkbox commit C, exact parent `C^ = B`, authorized post-edit snapshot Q, and exact diff; C
is an ancestor of final candidate F; and every T073-T218 receipt row is satisfied. The
T600/T601 record union names F and F's tree produced by T220 and its `(lane, validation)`
multiset passes T589's checked-in eight-row closed-profile validator exactly; T220 has proved
the same validator is wired to panel-request, seal, and merge-eligibility, and its missing,
extra, duplicate, unknown, wrong-lane, and conflated negative tests are green. T589's strict-binding suite also proves synchronized cross-candidate first requests yield
exactly one success and one durable fd-anchored reservation. Before no-replace publication the
recovery oracle is zero reservations; after publication but before wave-directory `fsync` it
is zero or one; after directory `fsync` it is exactly one and every retry refuses.
Fd-relative orphan cleanup leaves no temporary residue and durably syncs deletion. Injected
crashes around panel-request publication, failed closure, active-slot release, and successor
admission prove idempotent ordering, zero or one active slot, retained failed/request records,
and no failed retry or duplicate request. Same-candidate retry, alternate candidate while
active, post-request byte-identical history rebase, and evidence refresh are rejected at
panel, seal, and merge-eligibility. Table-driven malformed, stale, cross-candidate, and
cross-wave recovery evidence independently covers predecessor candidate, program/wave,
recommendations digest, successor commit/tree, convergence identity, and validation identity.
A distinct successor is admitted only after durable failed-candidate closure and complete
scoped recovery evidence; generic history-only reuse remains green before a candidate's
request or outside the strict profile. T604's
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

This amendment changes the reviewed Wave 5 plan boundary. The first eligible action is the
pre-T603 read-only analysis at A/P0, followed by a unanimous plan panel at A/P0. That pair
authorizes only the T603 validator commit V. After V, T603 freezes B exactly at V and reruns
analysis plus `/d2b-panel-round plan` with qualified wave `adr046w5` and a round address of the
form `adr046w5-r<n>`, both bound to B/P. Only the no-HIGH/CRITICAL post-validator analysis,
unanimous post-validator plan receipt, and finalized editor progress receipt permit
`/d2b-autopilot --resume` to continue from the `adr046w5` checkpoint. T603 still reconciles
exactly T073-T218; it records T605 only as future work after resume and does not add a 147th
receipt row or a 148th checkbox transition.

At wave close, T220 converges all content and freezes F before exact-candidate evidence.
T219 alone issues F's one binding panel request, attests, seals, and merges F. T220 never
invokes a binding panel. A pre-panel defect returns to T220 and invalidates F plus T600-T602
evidence before any request. After F's request, F and its evidence identity cannot change and
F cannot receive another request. A nonunanimous F remains immutable and failed; only its
recommendations enter the scoped fix round, which returns through T220 and T600-T602 and runs
a delta/full-context follow-up panel before a distinct successor receives its one request.
An external policy/tooling refusal blocks for integrator escalation rather than waiving a
finding. A successful merge preserves the selected candidate's tree.

### Recorded drift

`ADR-046-validation-and-delivery` §3.2 lists `packages/d2b-process/` and
`packages/d2b-provider-supervisor/` under W2. No W2 work item targets either path; the owning
item `ADR046-process-001` is W4. Per "existing code is canon" the machine-readable graph wins.
This plan follows the graph. Correcting the prose is a specification amendment that re-opens
that spec's evidence, so it is raised to the integrator rather than fixed mid-wave.

`ADR-046-validation-and-delivery` section 12.3 says the binding panel runs exactly once per
wave, while the repository panel recovery contract requires scoped fix and follow-up rounds
after recommendations. The current delivery tooling also has no candidate-scoped strict
reservation/recovery profile; T589 is future work and cannot govern W2-W4 retroactively.
The versioned feature-local
`contracts/README.md#candidate-recovery-prerequisite-v1` therefore made T008 the intended W2
entry owner. The committed history now contains downstream W2, W3, and W4 implementation while
T008, T030, and T037 remain unchecked. That is historical drift, not evidence that any gate
passed. All three tasks are reclassified as historical entry attestations: only exact retained
evidence from the actual first-dispatch base may check them. If that evidence is unavailable,
the task stays unchecked and the candidate must instead carry the named passing remedial
requalification record. T030's disposition is required before proposed F3 is declared frozen;
the established F2/F4 remedial paths remain required before their close gates. T029, T036,
and T071 each require exactly one historical or remedial disposition before panel request,
seal, or merge; absence or duplication fails closed. A current rerun never masquerades as
historical compliance. T589 consumes accepted v1 on its own actual base and adds the stricter
`adr046w5` storage profile;
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
validation obligations are met; T601 must measure the amended candidate and T602 rejects either
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
assigned to T605. No implementation is claimed, and implementation remains gated on the
pre-T603 A/P0 analysis/panel, T603's validator-only V/B commit, the post-T603 B/P
analysis/panel, and the receipt/editor transition.

### Program-local safety and delivery risks

These rows are not Constitution Principle VI deviations.

| Risk | Why Tracked | Guard and Rejected Alternative |
| --- | --- | --- |
| FR-043 (recovery-point attestation) is tracked program-local, outside the work-item manifest, so the manifest census alone cannot enforce it | FR-043 is locally added and **stricter** than `ADR-046-reset-and-cutover`, which permits proceeding past the rollback boundary without attestation. Creating a manifest work item would require amending that member spec, which re-opens its validation and panel evidence and re-triggers Gate 0. An unqualified "backup exists" assertion permits a partial, old, wrong-host, or unverifiable point to become success-shaped. | Keep it program-local, but close the safety gap at the W7 exit boundary. T548 owns one hermetic validator used unchanged by T580, T555, and T556. It decodes every timestamp through a bounded integer newtype, uses checked bounded expiration arithmetic, requires `previewed <= captured <= verified <= attested <= verifier-now < expires`, independently varies every receipt field and binding including operator and restore-instruction digests, and fails on listing failure, empty discovery, ignored tests, or skip. Before T580 records evidence, the integrator freezes the clean current W7 candidate and exact preview inventory. T580 accepts only one external version 1 record for a verified full-host snapshot or backup covering boot/system state, the active generation, the preview inventory, and preserved identity state. It binds candidate/commit/tree, preview, daily-driver host, operator, and restore instructions; imports only its digest and opaque locator through the existing `EvidenceRecord`; and rejects negative, fractional, future, out-of-range, overflow, stale, expired, or mismatched values. Every close stage invokes the same validator. Expiry durably fails that candidate and retains its records; T580 creates a distinct successor with fresh attestation and validation, and T555 permits that successor one panel request. No approval transfers. The external operator-owned backup/snapshot and restore mechanism remains outside this feature; no host implementation is claimed. |
| Pipelined dispatch can create successor rework when a predecessor panel finding invalidates in-flight work | Constitution 2.1.0 expressly authorizes implementation of wave N+1 to begin at 5 of 10 wave-N panel returns plus green integration. It is therefore current policy, not a constitution deviation. | Unanimity, roster, seal ordering, and merge ordering remain unchanged. The successor rebases onto the merged predecessor before its own panel, so no panel reviews a tree built on unreviewed contracts. Strict serialization was rejected because it adds idle time without strengthening the exit gate. FR-050 forbids citing rework as grounds to shorten a panel. |
