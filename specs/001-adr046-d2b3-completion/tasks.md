---
description: "Task list for completing the ADR-046 Provider control plane (d2b 3.0)"
---

# Tasks: Complete the ADR-046 Provider Control Plane (d2b 3.0)

**Input**: Design documents from `/specs/001-adr046-d2b3-completion/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/](./contracts/),
[spec-coverage.md](./spec-coverage.md)

## How this task list is organized

This is not a greenfield feature. The work is 531 remaining work items already sequenced by a
committed dependency graph, delivered under a wave contract with a hard gate between waves.
Tasks are therefore grouped by **wave** first, then by **parallel group**, because that is the
real dependency and gating structure. User-story mapping is recorded per wave.

Waves are **pipelined**: the next wave starts coding once 5 of 10 predecessor panels return and
integration tests pass, but panel, seal, and merge remain **strictly ordered**. Within a wave, parallel groups are file-disjoint by
construction and MUST be launched in the same coordination cycle - a ready slice left
unlaunched without a recorded blocker is a process failure, not a scheduling preference.

## Format: `[ID] [P?] [Story] WorkItemId - destination (reuseAction)`

- **[P]**: Free to start immediately within its wave - no incoming dependency or file-overlap
  edge. 100 of 545 items qualify; the rest wait on a named predecessor.
- **[Story]**: US1 live resource plane, US2 Providers, US3 cutover, US4 release.
- Destination shown is the **first** path only. The authoritative destination list, and every
  other obligation, lives in the manifest.

## The authoritative-detail rule

Each task below is a **pointer to a manifest entry, never a summary of one**. Before starting
any task, retrieve its full entry:

```bash
jq --arg id ADR046-routing-001 \
  '.items[] | select(.workItemId==$id)' \
  docs/specs/ADR-046-work-items.json
```

That entry carries `detailedDesign`, `validation`, the complete `destination` list,
`integration`, `dataMigration`, `currentSource`, `reuseAction`, `reuseSource`,
`dependencyOwner`, and `removalProof`. **Those fields are the task.** A task is not complete
until its `validation` obligations are satisfied and its `removalProof` passes where it
retires a path.

Deliberately, this file does not copy that text. Duplicating 531 manifest entries into
Markdown would create a second source of truth that no drift gate checks - the same failure
this program is trying to avoid. Reference, never paraphrase.

## Wave gate tasks

Every wave carries the same gate. Gate tasks are numbered inline with the wave they close.

---

## Phase 0: Pre-W2 spec hygiene (BLOCKING)

**Purpose**: Close the requirements defects that would otherwise be inherited by every wave.
Gate 1 items were closed during planning; these are the Gate 2 items that block declaring W2
entry criteria met.

- [X] T001 Resolve CHK013 - state Gate 0's standing re-evaluation obligation as a requirement, not only an assumption
- [X] T002 Resolve CHK027 - state the distinction between wave entry evidence and exit evidence so FR-025 and FR-036 do not read as conflicting
- [X] T003 Resolve CHK028 - fix the waiver scope for the nine `ADR046-delivery-*` items that remain Planned while owned by a waived wave
- [X] T004 Resolve CHK039 - state the contended-file prep discipline; W2 has a single `nixos-modules/assertions.nix` writer and this has immediate effect
- [X] T005 Record every Gate 3 checklist item as a deliberate deferral naming its owning wave, so a scheduled obligation is never mistaken for a coverage gap
- [X] T006 Answer CHK047 - confirm whether cloud accounts and access exist for the Azure-backed Provider validation required at W6 and by the release gate
- [X] T007 Prototype the RSS corrections (range-seek replay, streaming decode, shared immutable ChangeBatch fan-out) in `proofs/redb-resource-store-spike/` so W5 confirms rather than discovers (mitigates RK-1)
- [X] T574 **Author and record the W0/W1 delivered-without-seal waiver** (FR-034). It MUST name the missing artifacts (the ten panel receipts and the seal), state the evidence actually relied upon (all 14 assigned work items recorded as Merged, merged through reviewed pull requests), and exist before W2 entry is declared met. This is the sole mitigation for the tracked constitution Principle VI deviation; without it the deviation is undocumented in practice
- [X] T575 **Raise the recorded W2 destination drift to the integrator as a specification amendment** (FR-046). `ADR-046-validation-and-delivery` §3.2 lists `packages/d2b-process/` and `packages/d2b-provider-supervisor/` under W2, but the graph assigns their owning item `ADR046-process-001` to W4. Follow the graph; do not correct the prose inside a wave
- [X] T576 **Inventory which migration-map DELETE and REPLACE rows still lack a removal proof** and assign each missing proof to the wave that removes its path (FR-023). The map currently supplies explicit proofs for only 3 of its 16 DELETE rows

### Panel model migration (BLOCKING - no wave can seal until this lands)

Panel reviews run on `gemini-3.1-pro-preview`, dispatched as subagent lanes by the coding
agent executing this plan. The delivery tooling currently pins a different model in five
enforcing places, and `panel-attest` **rejects** any record whose model does not match. Until
these tasks land, every panel record is invalid and `seal` cannot succeed.

- [X] T581 Amend `ADR-046-validation-and-delivery` §12.3 to bind the panel to `gemini-3.1-pro-preview`, updating the pinned provider/model/reasoning-effort triple and the 14-field record example. This is a member-spec amendment: it re-opens that spec's validation and panel evidence and re-triggers Gate 0 (FR-046)
- [X] T582 Update the pinned constants in `packages/xtask/src/delivery/model.rs` (`PANEL_PROVIDER_POLICY`, `PANEL_MODEL_POLICY`, `PANEL_REASONING_EFFORT_POLICY`) and the unit test at the bottom of that file that asserts their exact values
- [X] T583 Update the `ADR046-delivery-005` work item text, which explicitly says "adapt to bind the fixed `gpt-5.6-sol` model at reasoning effort `xhigh`", then regenerate the spec-set and work-item manifests and confirm `make test-drift` is clean
- [X] T584 Add a read-only `panel` agent to `.opencode/opencode.json` pinned to the panel model, and correct the AGENTS.md panel-tooling wording, so panel lanes do not silently fall back to a model whose records `panel-attest` will reject. Spec correction: this task originally named `.opencode/opencode-swarm.json`, which does not exist in this repository and would not apply, because this program does not run swarm

### Pipelined-wave migration (BLOCKING - the pipeline is not executable until this lands)

Constitution 2.0.0 permits pipelined dispatch, but `ADR-046-validation-and-delivery` §4 still
reads "there is no partial-wave advance" and its entry criterion 1 still requires every prior
wave item to be `Merged` before `wave snapshot` will accept entry.

- [X] T585 Amend `ADR-046-validation-and-delivery` §4 to permit pipelined implementation start under the four conditions (5 of 10 reviews returned, integration green, no successor panel/seal/merge before predecessor seal and merge, mandatory post-merge rebase before the successor panel). Preserve the strict panel/seal/merge ordering verbatim. Member-spec amendment: re-opens that spec's evidence and re-triggers Gate 0 (FR-046)
- [X] T586 Relax the `wave snapshot` entry check so an unsealed predecessor blocks the successor's **exit boundary** rather than its implementation start; the predecessor-merged assertion moves to the exit boundary: `panel-request`, `seal`, and `merge-eligibility`. Add tests covering: start permitted at 5 of 10, panel request refused while the predecessor is unsealed, and seal refused when the successor has not rebased since the predecessor merge
- [X] T587 Record the accepted rework cost (FR-050) in the delivery contract so a future integrator cannot cite pipeline rework as grounds to shorten a panel
- [X] T588 Configure or document review scoping for the `v3` lineage. `detect-changed-files.sh` resolves the default branch to `main` via `origin/HEAD`, but ADR-046 integrates on `v3`, which never merges to `main`. Every wave review MUST pass an explicit diff scope (wave integration branch against its real base) or it will treat the whole v3 divergence as the wave changes

**Checkpoint**: W2 entry criteria may now be declared met.

---
## Wave W2: Primitive resource composition and Zone routing

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 19 | **Parallel groups**: 2

- [ ] T008 [US1] W2 ENTRY - confirm destinations uncontended, stack proposed against the exact parent commit, heavy-gate free, fast hermetic suite green. Note: under FR-057 and delivery contract §4, "every prior-wave work item is Merged" is **not** an entry criterion; it binds at the exit boundary - panel request, seal, and merge eligibility (T028)

### Group `wi:ADR-046-primitive-resource-composition` (3 items)

- [X] T009 [P] [US1] `ADR046-primitives-001` - `packages/d2b-contracts/src/v3/host.rs` (adapt)
- [X] T010 [P] [US1] `ADR046-primitives-002` - `packages/d2b-provider-system-systemd/` (adapt)
- [X] T011 [P] [US1] `ADR046-primitives-003` - `packages/d2b-provider-volume-*/` (adapt)

### Group `wi:ADR-046-zone-routing` (16 items)

- [X] T012 [P] [US1] `ADR046-routing-001` - `packages/d2b-contracts/src/v3/zone_routing.rs` (adapt)
- [X] T013 [US1] `ADR046-routing-002` - `packages/d2b-zone-routing/src/engine.rs` (adapt)
- [X] T014 [US1] `ADR046-routing-003` - `packages/d2b-zone-routing/src/resolver.rs` (ZoneEntrypointResolver) (adapt)
- [X] T015 [US1] `ADR046-routing-004` - `packages/d2b-core-controller/src/zone_links.rs` (adapt)
- [X] T016 [US1] `ADR046-routing-005` - `packages/d2b-bus/src/zone_route.rs` (cross-Zone bus routing) (adapt)
- [X] T017 [US1] `ADR046-routing-006` - `packages/d2b-zone-routing/tests/route_engine_vectors.rs` (adapt)
- [X] T018 [P] [US1] `ADR046-routing-007` - `packages/d2b-bus/src/session/` (adapt)
- [X] T019 [US1] `ADR046-routing-008` - `packages/d2b-bus/src/transport/unix.rs` (adapt)
- [X] T020 [US1] `ADR046-routing-009` - `packages/d2b-contracts/src/v3/zone_session.rs` (adapt)
- [X] T021 [US1] `ADR046-routing-010` - `packages/d2b-resource-client/` (adapt)
- [X] T022 [US1] `ADR046-routing-011` - `nixos-modules/options-zones.nix` (new structural base) (adapt)
- [X] T023 [US1] `ADR046-routing-012` - `nixos-modules/zone-resources-json.nix` (new) (adapt)
- [X] T024 [US1] `ADR046-routing-013` - `packages/d2b-core-controller/src/configuration.rs` (defined by ADR-046-core-controllers) (adapt)
- [X] T025 [US1] `ADR046-routing-014` - `packages/d2b-provider/src/` (adapted in place) (adapt)
- [X] T026 [US1] `ADR046-routing-015` - `packages/d2b-provider-toolkit/src/` (adapted in place) (adapt)
- [X] T027 [US1] `ADR046-routing-016` - `packages/d2b-zone-routing/src/service.rs` (adapt)

- [ ] T028 [US1] W2 GATE - run `/speckit.verify.run` and `/speckit.review.run` in parallel against this wave scope FIRST and clear their CRITICAL findings; then snapshot, import validation evidence, panel-request (refused unless every prior-wave work item is Merged, per FR-057), panel-attest (10/10 unanimous), seal (every prior-wave item and every wave item Merged), merge-target, merge-eligibility. Also confirm for every item in this wave: reference docs landed with their behavior (FR-019), no change contradicts a decision in the register (FR-047), and every removal proof for a path retired in this wave passed (FR-023). From round 9, LOW/MEDIUM may be deferred to deferred-findings.md; CRITICAL/HIGH never (FR-051, FR-052). At wave close, review both registers and log this wave friction in friction-log.md (FR-053)
- [ ] T029 [US1] W2 CONVERGE + MERGE - merge every slice branch into the wave integration branch; run integration tests, panel, and CI against the converged tree only; open one PR against `v3`; merge after eligibility; rebase the next wave onto the updated `v3`; fold changelog fragments; then clean up in order: delete each worktree packages/target, remove worktrees, delete local branches, delete remote branches, nix-collect-garbage, and audit `git worktree list` plus `git branch -a` for residue

**Checkpoint**: W2 converged, panelled, sealed, merged to `v3`, rebased, and cleaned up. Successor entry criteria satisfied.

---

## Wave W3: Provider model and packaging (strictly serial - gates every dossier)

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 4 | **Parallel groups**: 1

- [ ] T030 [US1] W3 ENTRY - confirm every prior-wave work item is Merged, destinations uncontended, stack proposed against the exact parent commit, heavy-gate free, fast hermetic suite green. Note: under FR-057 and delivery contract §4, "every prior-wave work item is Merged" is **not** an entry criterion; it binds at the exit boundary - panel request, seal, and merge eligibility (T035)

### Group `wi:ADR-046-provider-model-and-packaging` (4 items)

- [ ] T031 [P] [US1] `ADR046-provider-001` - `packages/d2b-contracts/src/v3/provider.rs` (adapt)
- [ ] T032 [P] [US1] `ADR046-provider-002` - one `packages/d2b-provider-<base>-<implementation>/` per Provider with mandatory src/ (adapt)
- [ ] T033 [P] [US1] `ADR046-provider-003` - `packages/d2b-provider-system-core/` (adapt)
- [ ] T034 [US1] `ADR046-provider-004` - `packages/d2b-contracts/src/v3/semantic_services/{mod (create)

- [ ] T035 [US1] W3 GATE - run `/speckit.verify.run` and `/speckit.review.run` in parallel against this wave scope FIRST and clear their CRITICAL findings; then snapshot, import validation evidence, panel-request, panel-attest (10/10 unanimous), seal (every wave item Merged), merge-target, merge-eligibility. Also confirm for every item in this wave: reference docs landed with their behavior (FR-019), no change contradicts a decision in the register (FR-047), and every removal proof for a path retired in this wave passed (FR-023). From round 9, LOW/MEDIUM may be deferred to deferred-findings.md; CRITICAL/HIGH never (FR-051, FR-052). At wave close, review both registers and log this wave friction in friction-log.md (FR-053)
- [ ] T036 [US1] W3 CONVERGE + MERGE - merge every slice branch into the wave integration branch; run integration tests, panel, and CI against the converged tree only; open one PR against `v3`; merge after eligibility; rebase the next wave onto the updated `v3`; fold changelog fragments; then clean up in order: delete each worktree packages/target, remove worktrees, delete local branches, delete remote branches, nix-collect-garbage, and audit `git worktree list` plus `git branch -a` for residue

**Checkpoint**: W3 converged, panelled, sealed, merged to `v3`, rebased, and cleaned up. Successor entry criteria satisfied.

---

## Wave W4: Components/processes/sandbox, core controllers, provider state, network and credential resources

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 32 | **Parallel groups**: 6

- [ ] T037 [US1] W4 ENTRY - confirm every prior-wave work item is Merged, destinations uncontended, stack proposed against the exact parent commit, heavy-gate free, fast hermetic suite green

### Group `wi:ADR-046-components-processes-and-sandbox` (2 items)

- [ ] T038 [P] [US1] `ADR046-process-001` - `packages/d2b-process/src/` (adapt)
- [ ] T039 [US1] `ADR046-process-002` - `packages/d2b-provider-system-systemd/` (adapt)

### Group `wi:ADR-046-core-controllers` (1 items)

- [ ] T040 [P] [US1] `ADR046-core-001` - `packages/d2b-core-controller/src/{main,configuration,api_catalog,authz,providers,controllers,ownership,watches,cleanup,zone_links,budgets,store}.rs` (adapt)

### Group `wi:ADR-046-provider-state` (12 items)

- [ ] T041 [US1] `ADR046-pstate-001` - `packages/d2b-contracts/src/v3/volume_state.rs` (adapt)
- [ ] T042 [US1] `ADR046-pstate-002` - `packages/d2b-contracts/src/v3/provider.rs` (component descriptor `stateNamespaces` field) (adapt)
- [ ] T043 [US1] `ADR046-pstate-003` - `packages/d2b-provider-volume-local/` (new crate (adapt)
- [ ] T044 [US1] `ADR046-pstate-004` - `packages/d2b-provider-volume-local/src/migration.rs` (adapt)
- [ ] T045 [US1] `ADR046-pstate-005` - `packages/d2b-provider-volume-local/src/sealing.rs` (adapt)
- [ ] T046 [US1] `ADR046-pstate-006` - `packages/d2b-provider-volume-local/src/snapshot.rs` (adapt)
- [ ] T047 [US1] `ADR046-pstate-007` - `packages/d2b-provider-volume-local/src/relocation.rs` (adapt)
- [ ] T048 [US1] `ADR046-pstate-008` - `packages/d2b-provider-volume-local/src/audit.rs` (adapt)
- [ ] T049 [US1] `ADR046-pstate-009` - `packages/d2b-provider-volume-local/tests/state.rs` (ported hermetic atomic/lock/quarantine/lease tests) (adapt)
- [ ] T050 [US1] `ADR046-pstate-010` - `nixos-modules/zone-resources.nix` (per-Zone bundle emitter NixOS module) (adapt)
- [ ] T051 [US1] `ADR046-pstate-011` - `packages/xtask/src/provider_crate_policy.rs` (adapt)
- [ ] T052 [US1] `ADR046-pstate-012` - `packages/d2b-core-controller/src/optional_state_admission.rs` (storage-need admission: reject a declared namespace whose payload is derivable from spec/status/core ledger/external observation with `component-state-not-justified` (adapt)

### Group `wi:ADR-046-resources-credential` (8 items)

- [ ] T053 [US1] `ADR046-credential-001` - `packages/d2b-contracts/src/v3/credential.rs` (adapt)
- [ ] T054 [US1] `ADR046-credential-002` - `packages/d2b-contracts/proto/v3/credential.proto` (adapt)
- [ ] T055 [US1] `ADR046-credential-003` - `packages/d2b-provider-credential-secret-service/src/{lib.rs (adapt)
- [ ] T056 [US1] `ADR046-credential-004` - `packages/d2b-provider-credential-entra/src/{lib.rs (adapt)
- [ ] T057 [US1] `ADR046-credential-005` - `packages/d2b-provider-credential-managed-identity/src/{lib.rs (adapt)
- [ ] T058 [US1] `ADR046-credential-006` - `packages/d2b-provider-credential-<impl>/src/controller.rs` (adapt)
- [ ] T059 [US1] `ADR046-credential-007` - `nixos-modules/options-resources.nix` (generic schema-derived resource options (adapt)
- [ ] T060 [US1] `ADR046-credential-008` - `packages/d2b-provider-credential-<impl>/src/audit.rs` (adapt)

### Group `wi:ADR-046-resources-network` (8 items)

- [ ] T061 [P] [US1] `ADR046-network-001` - `packages/d2b-contracts/src/v3/network.rs`: NetworkSpec (adapt)
- [ ] T062 [US1] `ADR046-network-002` - `packages/d2b-provider-network-local/src/ifname.rs` (adapt)
- [ ] T063 [US1] `ADR046-network-003` - `packages/d2b-provider-network-local/` - artifact catalog integration for net-VM nixos-system artifact resolution (adapt)
- [ ] T064 [US1] `ADR046-network-004` - `nixos-modules/resources-network.nix`: Nix resource object emitter for Network ResourceType (adapt)
- [ ] T065 [US1] `ADR046-network-005` - `packages/d2b-provider-network-local/src/controller.rs`: async NetworkReconciler (adapt)
- [ ] T066 [US1] `ADR046-network-006` - `tests/unit/nix/cases/net-vm-network.nix` (adapted to v3 resource API) (adapt)
- [ ] T067 [US1] `ADR046-network-007` - `Provider/device-usbip` owns one relay Process/Endpoint authority per Network and calls the typed UsbipEffectPort for the shared closed `ApplyNftablesProjection` request with closed action enum `Apply/Remove` (adapt)
- [ ] T068 [US1] `ADR046-network-009` - `packages/d2b-contracts/src/v3/network.rs` external-attachment sharing schema/status (adapt)

### Group `wi:core-config-hub:w4` (1 items)

- [ ] T069 [US1] `ADR046-network-008` - `packages/d2b-core-controller/src/configuration.rs`: bundle application (create)

- [ ] T070 [US1] W4 GATE - run `/speckit.verify.run` and `/speckit.review.run` in parallel against this wave scope FIRST and clear their CRITICAL findings; then snapshot, import validation evidence, panel-request, panel-attest (10/10 unanimous), seal (every wave item Merged), merge-target, merge-eligibility. Also confirm for every item in this wave: reference docs landed with their behavior (FR-019), no change contradicts a decision in the register (FR-047), and every removal proof for a path retired in this wave passed (FR-023). From round 9, LOW/MEDIUM may be deferred to deferred-findings.md; CRITICAL/HIGH never (FR-051, FR-052). At wave close, review both registers and log this wave friction in friction-log.md (FR-053)
- [ ] T071 [US1] W4 CONVERGE + MERGE - merge every slice branch into the wave integration branch; run integration tests, panel, and CI against the converged tree only; open one PR against `v3`; merge after eligibility; rebase the next wave onto the updated `v3`; fold changelog fragments; then clean up in order: delete each worktree packages/target, remove worktrees, delete local branches, delete remote branches, nix-collect-garbage, and audit `git worktree list` plus `git branch -a` for residue

**Checkpoint**: W4 converged, panelled, sealed, merged to `v3`, rebased, and cleaned up. Successor entry criteria satisfied.

---

## Wave W5: Production store engine and watch, resource catalog, telemetry, CLI, Nix configuration

**Requirements**: see spec-coverage.md traceability tables | **Story**: US1 | **Work items**: 146 | **Parallel groups**: 12

- [ ] T072 [US1] W5 ENTRY - confirm every prior-wave work item is Merged, destinations uncontended, stack proposed against the exact parent commit, heavy-gate free, fast hermetic suite green

### Group `wi:ADR-046-cli-and-operations` (13 items)

- [ ] T073 [US1] `ADR046-cli-001` - `packages/d2b/src/lib.rs` (adapt)
- [ ] T074 [US1] `ADR046-cli-002` - `packages/d2b/src/guest.rs` (`d2b guest start/stop/restart/list/status`) (adapt)
- [ ] T075 [US1] `ADR046-cli-003` - `packages/d2b/src/exec.rs` (`d2b exec run/attach/wait/status/list/logs/kill`) (adapt)
- [ ] T076 [US1] `ADR046-cli-004` - `packages/d2b/src/shell.rs` (`d2b shell open/attach/list/detach/kill/status`) (adapt)
- [ ] T077 [US1] `ADR046-cli-005` - `packages/d2b/src/provider.rs` (`d2b provider list/get/status/inspect` (adapt)
- [ ] T078 [US1] `ADR046-cli-006` - `packages/d2b/src/complete.rs` (`d2b complete bash/zsh/fish`) (adapt)
- [ ] T079 [US1] `ADR046-cli-007` - `packages/d2b/src/activation.rs` (`d2b activation build/generations/switch/boot/test/rollback/gc/migrate/keys/trust/rotate-known-host/config`) (adapt)
- [ ] T080 [US1] `ADR046-cli-008` - `packages/d2b/src/host.rs` (all `d2b host` subcommands) (adapt)
- [ ] T081 [US1] `ADR046-cli-009` - `packages/d2b/src/zone.rs` (`d2b zone get/list/status`) (adapt)
- [ ] T082 [US1] `ADR046-cli-010` - `packages/d2b/src/resource.rs` (standard `d2b get/list/watch/create/update-spec/delete/status` top-level verbs) (adapt)
- [ ] T083 [US1] `ADR046-cli-011` - Nix: `nixos-modules/options-zones.nix` (unified `d2b.zones.<zone>.resources` attrset (replace)
- [ ] T084 [US1] `ADR046-cli-012` - `packages/d2b/src/endpoint.rs` (`d2b endpoint get/list/watch/status/resolve`) (adapt)
- [ ] T085 [US1] `ADR046-cli-013` - `packages/d2b/src/share.rs` (`d2b export …` and `d2b import …` nouns) (adapt)

### Group `wi:ADR-046-nix-configuration` (35 items)

- [ ] T086 [US1] `ADR046-nix-001` - `nixos-modules/options-zones.nix` (Zone-level options: `label` (adapt)
- [ ] T087 [US1] `ADR046-nix-002` - `Network` resource fields in `nixos-modules/options-zones-resources.nix` (adapt)
- [ ] T088 [US1] `ADR046-nix-003` - `nixos-modules/options-site.nix` (retained) (adapt)
- [ ] T089 [US1] `ADR046-nix-004` - `nixos-modules/index.nix` (rewritten) (adapt)
- [ ] T090 [US1] `ADR046-nix-005` - `nixos-modules/bundle-zones.nix` (per-Zone bundle derivation) (adapt)
- [ ] T091 [US1] `ADR046-nix-006` - `nixos-modules/resources-zones-processes.nix` (adapt)
- [ ] T092 [US1] `ADR046-nix-007` - `nixos-modules/resources-zones-volumes.nix` (adapt)
- [ ] T093 [US1] `ADR046-nix-008` - Compiler-only `parentZone` map in `nixos-modules/options-zones.nix` (adapt)
- [ ] T094 [US1] `ADR046-nix-009` - Provider/display-wayland and Provider/shell-terminal Process configs in `zones/<z>/resource-bundle.json` (adapt)
- [ ] T095 [US1] `ADR046-nix-010` - User-only `Host` resource in `zones/<z>/resource-bundle.json` (`spec.isolationPosture: "none"` (adapt)
- [ ] T096 [P] [US1] `ADR046-nix-011` - `nixos-modules/privileges-json.nix` (retained) (copy-unchanged)
- [ ] T097 [US1] `ADR046-nix-012` - `nixos-modules/closures-json.nix` (rewritten (adapt)
- [ ] T098 [US1] `ADR046-nix-013` - Per-Zone `zones/<z>/resource-bundle.json` (`schemaVersion`) (replace)
- [ ] T099 [US1] `ADR046-nix-014` - `nixos-modules/assertions.nix` (adapt)
- [ ] T100 [US1] `ADR046-nix-015` - Same files (adapt)
- [ ] T101 [US1] `ADR046-nix-016` - Network reconciliation by `Provider/network-local` Process resources (copy-unchanged)
- [ ] T102 [US1] `ADR046-nix-017` - Per-VM store reconciliation by `Provider/volume-virtiofs` EphemeralProcess/Process resources (copy-unchanged)
- [ ] T103 [US1] `ADR046-nix-018` - `Provider/device-tpm` (replace)
- [ ] T104 [US1] `ADR046-nix-019` - `docs/reference/schemas/v3/<ResourceType>.json` for each ResourceType (adapt)
- [ ] T105 [US1] `ADR046-nix-020` - Configuration-publication controller handler in `packages/d2b-core-controller/src/configuration.rs` (create)
- [ ] T106 [US1] `ADR046-nix-021` - `packages/d2b-contract-tests/tests/provider-crate-layout.rs` (create)
- [ ] T107 [US1] `ADR046-nix-022` - `nixos-modules/artifact-catalog.nix` (new emitter) (create)
- [ ] T108 [US1] `ADR046-nix-023` - `packages/d2b-bus/src/session/` (new crate `d2b-bus`) (adapt)
- [ ] T109 [US1] `ADR046-nix-024` - `packages/d2b-bus/src/session/` (same crate as ADR046-nix-023). (adapt)
- [ ] T110 [US1] `ADR046-nix-025` - `packages/d2b-bus/src/session/`. (adapt)
- [ ] T111 [US1] `ADR046-nix-026` - `packages/d2b-bus/src/transport/unix/`. (adapt)
- [ ] T112 [P] [US1] `ADR046-nix-027` - `packages/d2b-contracts/src/v3/component_session.rs`. (adapt)
- [ ] T113 [US1] `ADR046-nix-028` - `packages/d2b-contracts/src/v3/services/`. (adapt)
- [ ] T114 [US1] `ADR046-nix-029` - `packages/d2b-provider/src/` (adapt in place). (adapt)
- [ ] T115 [US1] `ADR046-nix-030` - `packages/d2b-provider-toolkit/src/` (adapt in place). (adapt)
- [ ] T116 [US1] `ADR046-nix-031` - `nixos-modules/resources-sharing.nix` (create)
- [ ] T117 [US1] `ADR046-nix-032` - `packages/d2b-client/src/` (adapt in place). (adapt)
- [ ] T118 [US1] `ADR046-nix-033` - `packages/d2b-bus/src/routing/zone_service.rs`. (adapt)
- [ ] T119 [US1] `ADR046-nix-034` - `packages/d2bd/src/provider_registry.rs` (adapt in place). (adapt)
- [ ] T120 [US1] `ADR046-nix-035` - `packages/d2bd/src/provider_effects.rs` (adapt in place). (adapt)

### Group `wi:ADR-046-resources-device` (7 items)

- [ ] T121 [P] [US1] `ADR046-device-001` - `packages/d2b-contracts/src/v3/device.rs` (adapt)
- [ ] T122 [US1] `ADR046-device-002` - `packages/d2b-provider-device-tpm/src/` (controller (adapt)
- [ ] T123 [US1] `ADR046-device-003` - `packages/d2b-provider-device-usbip/src/` (controller (adapt)
- [ ] T124 [US1] `ADR046-device-004` - `packages/d2b-provider-device-security-key/src/` (controller (adapt)
- [ ] T125 [US1] `ADR046-device-005` - `packages/d2b-provider-device-gpu/src/` (controller (adapt)
- [ ] T126 [US1] `ADR046-device-006` - `nixos-modules/resources-device.nix` (adapt)
- [ ] T127 [US1] `ADR046-device-008` - `packages/xtask/src/main.rs` (`check-provider-layout` subcommand) (adapt)

### Group `wi:ADR-046-resources-host-guest-process-user` (22 items)

- [ ] T128 [P] [US1] `ADR046-exec-001` - `packages/d2b-contracts/src/v3/host.rs` (adapt)
- [ ] T129 [US1] `ADR046-exec-002` - `packages/d2b-contracts/src/v3/process_provider.rs`: LaunchTicket (adapt)
- [ ] T130 [US1] `ADR046-exec-003` - `packages/d2b-provider-system-core/src/host.rs`: HostReconciler (adapt)
- [ ] T131 [US1] `ADR046-exec-004` - `packages/d2b-provider-system-core/src/user.rs`: UserReconciler (adapt)
- [ ] T132 [US1] `ADR046-exec-005` - `packages/d2b-provider-system-core/src/host.rs` (continued) (adapt)
- [ ] T133 [US1] `ADR046-exec-006` - `packages/d2b-provider-system-systemd/src/`: launch.rs (opaque EffectPort requests) (adapt)
- [ ] T134 [US1] `ADR046-exec-007` - `packages/d2b-provider-system-minijail/src/`: sandbox_compiler.rs (adapt)
- [ ] T135 [US1] `ADR046-exec-008` - `packages/d2b-process-conformance/src/`: shared conformance test matrix run against both system-systemd and system-minijail providers (adapt)
- [ ] T136 [US1] `ADR046-exec-009` - `packages/d2b-provider-system-core/src/host.rs` (user-only no-isolation Host) (adapt)
- [ ] T137 [US1] `ADR046-exec-010` - `packages/d2b-provider-system-systemd/src/guest_exec.rs` (guest-domain EphemeralProcess launch via systemd-run inside guest) (adapt)
- [ ] T138 [US1] `ADR046-exec-011` - guest-domain process attachment becomes a ComponentSession named stream to the EphemeralProcess running in the guest (adapt)
- [ ] T139 [US1] `ADR046-exec-012` - `nixos-modules/options-zones.nix`: `d2b.zones.<zone>.resources` option as `types.attrsOf (types.submodule resourceModule)` where each resource module has `type` (required enum) (adapt)
- [ ] T140 [US1] `ADR046-exec-014` - `nixos-modules/zone-bundle.nix`: Zone resource bundle emitter (adapt)
- [ ] T141 [US1] `ADR046-exec-016` - `packages/d2b-bus-session/src/`: all above modules verbatim (adapt)
- [ ] T142 [US1] `ADR046-exec-017` - `packages/d2b-bus-session-unix/src/`: all above modules verbatim (adapt)
- [ ] T143 [US1] `ADR046-exec-018` - `packages/d2b-bus-wire/src/session.rs`: v3 bus protocol constants and wire types (adapt)
- [ ] T144 [US1] `ADR046-exec-019` - `packages/d2b-provider-runtime/src/`: `registry.rs` (adapt)
- [ ] T145 [US1] `ADR046-exec-020` - `packages/d2b-provider-toolkit/src/`: retain all modules verbatim (adapt)
- [ ] T146 [US1] `ADR046-exec-021` - `packages/d2b-bus-contracts/src/generated_v3_services/`: v3 generated ttrpc stubs for Zone service methods (Resource CRUD (adapt)
- [ ] T147 [US1] `ADR046-exec-022` - `packages/d2b-bus-client/src/`: all above modules (adapt)
- [ ] T148 [US1] `ADR046-exec-023` - `packages/d2b-zone-router/src/`: `router.rs` (v3 `ZoneOperationRouter` - idempotency semantics copied verbatim (adapt)
- [ ] T149 [US1] `ADR046-user-session-001` - `packages/d2b-core-controller/src/user_session_authority.rs` (or a core/user-agent per-session agent Process under `Provider/system-systemd`) (adapt)

### Group `wi:ADR-046-resources-volume` (6 items)

- [ ] T150 [P] [US1] `ADR046-volume-001` - `packages/d2b-contracts/src/v3/volume.rs` (adapt)
- [ ] T151 [US1] `ADR046-volume-002` - `packages/d2b-provider-volume-local/src/` (layout engine (adapt)
- [ ] T152 [US1] `ADR046-volume-003` - `packages/d2b-provider-volume-virtiofs/src/` (controller (adapt)
- [ ] T153 [US1] `ADR046-volume-004` - `nixos-modules/resources-volume.nix` (adapt)
- [ ] T154 [US1] `ADR046-volume-005` - `packages/d2b-provider-volume-local/src/` (block-image (create)
- [ ] T155 [US1] `ADR046-volume-006` - `nixos-modules/resources-volume.nix` (Nix eval-time schema validation (create)

### Group `wi:ADR-046-resources-zone-control` (26 items)

- [ ] T156 [US1] `ADR046-client-001` - `packages/d2b-client/src/` (updated for v3 Zone API (adapt)
- [ ] T157 [US1] `ADR046-pkg-001` - `packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs` (new file (create)
- [ ] T158 [US1] `ADR046-provider-agent-001` - `packages/d2b-provider/src/agent.rs` (v3 provider agent dispatch) (adapt)
- [ ] T159 [US1] `ADR046-wire-001` - `packages/d2b-contracts/src/v3/{services (adapt)
- [ ] T160 [US1] `ADR046-zone-control-001` - `packages/d2b-contracts/src/v3/zone.rs` (adapt)
- [ ] T161 [US1] `ADR046-zone-control-002` - `packages/d2b-contracts/src/v3/zone_link.rs` (adapt)
- [ ] T162 [US1] `ADR046-zone-control-003` - `packages/d2b-contracts/src/v3/provider.rs` (adapt)
- [ ] T163 [US1] `ADR046-zone-control-004` - `packages/d2b-contracts/src/v3/role.rs` (adapt)
- [ ] T164 [US1] `ADR046-zone-control-005` - `packages/d2b-contracts/src/v3/role_binding.rs` (adapt)
- [ ] T165 [US1] `ADR046-zone-control-006` - `packages/d2b-resource-api/src/authz.rs` (adapt)
- [ ] T166 [US1] `ADR046-zone-control-007` - `nixos-modules/options-zones.nix` (adapt)
- [ ] T167 [US1] `ADR046-zone-control-008` - `packages/d2b-contracts/src/v3/host.rs` (Host resource schema (adapt)
- [ ] T168 [US1] `ADR046-zone-control-009` - `packages/d2b-contracts/src/v3/quota.rs` (create)
- [ ] T169 [US1] `ADR046-zone-control-010` - `packages/d2b-contracts/src/v3/emergency_policy.rs` (create)
- [ ] T170 [US1] `ADR046-zone-control-011` - `packages/d2b-bus/src/{lifecycle (adapt)
- [ ] T171 [US1] `ADR046-zone-control-012` - `packages/d2b-bus-unix/src/{adapter (adapt)
- [ ] T172 [US1] `ADR046-zone-control-013` - `packages/d2b-contracts/src/v3/component_session.rs` (new v3 namespace in existing contracts crate) (adapt)
- [ ] T173 [US1] `ADR046-zone-control-014` - `nixos-modules/options-zones.nix` (create)
- [ ] T174 [US1] `ADR046-zone-control-015` - `packages/d2b-resource-compiler/src/{main (create)
- [ ] T175 [US1] `ADR046-zone-control-017` - `packages/d2b-provider/src/{registry (adapt)
- [ ] T176 [US1] `ADR046-zone-control-018` - `packages/d2b-core-controller/src/zone_link.rs` (ZoneLink handler) (adapt)
- [ ] T177 [US1] `ADR046-zone-control-019` - `packages/d2b-contracts/src/v3/{resource_export (adapt)
- [ ] T178 [US1] `ADR046-zone-control-020` - `packages/d2b-core-controller/src/export_import_projection.rs` (local qualified Service projection lifecycle owned by `ResourceImport`) (create)
- [ ] T179 [US1] `ADR046-zone-control-022` - `packages/d2b-core-controller/src/authority.rs` (adapt)
- [ ] T180 [US1] `ADR046-zone-control-023` - `packages/d2b-core-controller/src/{quota (adapt)
- [ ] T181 [US1] `ADR046-zone-control-024` - `packages/d2b-core-controller/src/authority.rs` (Host-global index scope + hardware admission) (adapt)

### Group `wi:ADR-046-telemetry-audit-and-support` (26 items)

- [ ] T182 [P] [US1] `ADR046-audit-001` - `packages/d2b-audit/src/{hash_chain.rs (adapt)
- [ ] T183 [US1] `ADR046-audit-002` - `packages/d2b-resource-store-redb/src/audit.rs` (adapt)
- [ ] T184 [US1] `ADR046-audit-003` - `packages/d2b-session/src/audit.rs` (adapt)
- [ ] T185 [US1] `ADR046-audit-004` - `packages/d2b/src/zone_audit.rs` (new `d2b zone audit export` subcommand) (adapt)
- [ ] T186 [US1] `ADR046-doctor-001` - `packages/d2b/src/zone_doctor.rs` (adapt)
- [ ] T187 [US1] `ADR046-doctor-002` - `packages/d2b/src/zone_support_bundle.rs` (adapt)
- [ ] T188 [US1] `ADR046-host-posture-001` - `packages/d2b-provider-system-core/src/{host_reconciler.rs (adapt)
- [ ] T189 [US1] `ADR046-reuse-001` - `packages/d2b-session/` copied verbatim (adapt)
- [ ] T190 [US1] `ADR046-reuse-002` - `packages/d2b-session-unix/` copied verbatim. (adapt)
- [ ] T191 [US1] `ADR046-reuse-003` - `packages/d2b-client/` copied (adapt)
- [ ] T192 [US1] `ADR046-reuse-004` - `packages/d2b-provider/` and `packages/d2b-provider-toolkit/` copied with v3 session admission and bus routing adaptations. (adapt)
- [ ] T193 [US1] `ADR046-reuse-005` - `packages/d2b-provider-observability-otel/src/agent.rs` adapted (adapt)
- [ ] T194 [US1] `ADR046-reuse-006` - `packages/d2b-bus/src/routing.rs` adapted from `service_v2.rs` (adapt)
- [ ] T195 [US1] `ADR046-reuse-007` - `packages/d2b-bus/src/service_router.rs` and `packages/d2b-core-controller/src/provider_effects.rs`. (adapt)
- [ ] T196 [US1] `ADR046-reuse-008` - `packages/d2b-contract-tests/tests/component_session_v2_vectors.rs` and `tests/noise_vectors.rs` copied verbatim. (adapt)
- [ ] T197 [US1] `ADR046-reuse-009` - `packages/d2b-telemetry/src/session_metrics_sink.rs`. (adapt)
- [ ] T198 [P] [US1] `ADR046-telem-001` - `packages/d2b-telemetry/src/{trace_context.rs (adapt)
- [ ] T199 [US1] `ADR046-telem-002` - `packages/d2b-resource-store-redb/src/metrics.rs` (adapt)
- [ ] T200 [US1] `ADR046-telem-003` - `packages/d2b-resource-api/src/metrics.rs` (adapt)
- [ ] T201 [US1] `ADR046-telem-004` - `packages/d2b-core-controller/src/metrics.rs` (adapt)
- [ ] T202 [US1] `ADR046-telem-005` - `packages/d2b-provider-supervisor/src/metrics.rs` (adapt)
- [ ] T203 [US1] `ADR046-telem-006` - `packages/d2b-provider-observability-otel/src/` (adapt)
- [ ] T204 [US1] `ADR046-telem-007` - `packages/d2b-provider-observability-otel/src/nix/journald.nix` (new Nix fragment) (adapt)
- [ ] T205 [US1] `ADR046-telem-008` - `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` (new) (adapt)
- [ ] T206 [P] [US1] `ADR046-telem-009` - `nixos-modules/resources.nix` (uniform `d2b.zones.<zone>.resources` schema-aware option (adapt)
- [ ] T207 [US1] `ADR046-telem-010` - `nixos-modules/resources-bundle.nix` (build-time validation step 4 in the `resources-bundle` derivation) (adapt)

### Group `wi:core-config-hub:w5` (6 items)

- [ ] T208 [US1] `ADR046-device-007` - `packages/d2b-core-controller/src/configuration.rs` (create)
- [ ] T209 [US1] `ADR046-exec-013` - `packages/d2b-core-controller/src/cleanup.rs`: EphemeralProcess TTL cleanup controller handler (create)
- [ ] T210 [US1] `ADR046-exec-015` - `packages/d2b-core-controller/src/configuration.rs`: `ZoneConfigController` (create)
- [ ] T211 [US1] `ADR046-telem-011` - `packages/d2b-core-controller/src/{configuration.rs (adapt)
- [ ] T212 [US1] `ADR046-zone-control-016` - `packages/d2b-core-controller/src/configuration.rs` (Phase 3 activation (adapt)
- [ ] T213 [US1] `ADR046-zone-control-021` - `packages/d2b-core-controller/src/{coordinator (adapt)

### Group `wi:reconciliation-real-backend:w5` (1 items)

- [ ] T214 [US1] `ADR046-reconcile-003` - `packages/d2b-controller-toolkit/benches/reaction.rs` (adapt)

### Group `wi:resource-store-backend:w5` (1 items)

- [ ] T215 [US1] `ADR046-store-004` - `packages/d2b-resource-store-redb/src/lib.rs` (adapt)

### Group `wi:resource-store-integration:w5` (2 items)

- [ ] T216 [US1] `ADR046-store-003` - `packages/d2b-contracts/src/v3/storage.rs` (adapt)
- [ ] T217 [US1] `ADR046-store-005` - `packages/d2b-resource-store-redb/src/backup.rs` (adapt)

### Group `wi:resource-store-watch:w5` (1 items)

- [ ] T218 [US1] `ADR046-store-002` - `packages/d2b-resource-store-redb/src/revision_log.rs` (adapt)

- [ ] T577 [US1] **Publish the desktop-companion inventory** as a versioned reference document naming each companion, its exact consumed surface, and its verification status (FR-039, contracts/companion-contracts.md CO-1). Published at W5, not at release, so companions have time to adapt
- [ ] T578 [US1] **Publish the replacement contracts the companions consume**, early enough for them to adapt given that no preview release may be published (contracts/companion-contracts.md CO-2, FR-045)
- [ ] T579 [US1] **Resolve the FR-039 / FR-045 tension before these contracts publish** (CHK025). FR-039 blocks release on external repositories while FR-045 forbids the preview build they would adapt against. This is the last moment the choice is cheap: resolve it here or amend FR-045

- [ ] T219 [US1] W5 GATE - run `/speckit.verify.run` and `/speckit.review.run` in parallel against this wave scope FIRST and clear their CRITICAL findings; then snapshot, import validation evidence, panel-request, panel-attest (10/10 unanimous), seal (every wave item Merged), merge-target, merge-eligibility. Also confirm for every item in this wave: reference docs landed with their behavior (FR-019), no change contradicts a decision in the register (FR-047), and every removal proof for a path retired in this wave passed (FR-023). From round 9, LOW/MEDIUM may be deferred to deferred-findings.md; CRITICAL/HIGH never (FR-051, FR-052). At wave close, review both registers and log this wave friction in friction-log.md (FR-053)
- [ ] T220 [US1] W5 CONVERGE + MERGE - merge every slice branch into the wave integration branch; run integration tests, panel, and CI against the converged tree only; open one PR against `v3`; merge after eligibility; rebase the next wave onto the updated `v3`; fold changelog fragments; then clean up in order: delete each worktree packages/target, remove worktrees, delete local branches, delete remote branches, nix-collect-garbage, and audit `git worktree list` plus `git branch -a` for residue

**Checkpoint**: W5 converged, panelled, sealed, merged to `v3`, rebased, and cleaned up. Successor entry criteria satisfied.

---

## Wave W6: All 27 Provider dossiers in five file-disjoint families

**Requirements**: see spec-coverage.md traceability tables | **Story**: US2 | **Work items**: 257 | **Parallel groups**: 28

- [ ] T221 [US2] W6 ENTRY - confirm every prior-wave work item is Merged, destinations uncontended, stack proposed against the exact parent commit, heavy-gate free, fast hermetic suite green

### Group `wi:ADR-046-provider-activation-nixos` (7 items)

- [ ] T222 [P] [US2] `ADR046-activation-001` - packages/d2b-host/src/bin/d2b-activation-helper.rs (adapt)
- [ ] T223 [P] [US2] `ADR046-activation-002` - docs/reference/schemas/v3/activation-nixos.d2bus.org.NixosGeneration.json and packages/d2b-contracts/src/activation_nixos.rs (create)
- [ ] T224 [US2] `ADR046-activation-003` - packages/d2b-provider-activation-nixos/src/controller/ (replace)
- [ ] T225 [US2] `ADR046-activation-004` - packages/d2b-provider-activation-nixos/src/runner/ (adapt)
- [ ] T226 [P] [US2] `ADR046-activation-005` - packages/d2b/src/activation.rs (replace)
- [ ] T227 [P] [US2] `ADR046-activation-006` - nixos-modules/providers/activation-nixos.nix (adapt)
- [ ] T228 [US2] `ADR046-activation-007` - packages/d2b/src/lib.rs (delete-after-cutover)

### Group `wi:ADR-046-provider-audio-pipewire` (13 items)

- [ ] T229 [P] [US2] `ADR046-audio-001` - `packages/d2b-provider-audio-pipewire/src/audio_policy.rs` (copy-unchanged)
- [ ] T230 [US2] `ADR046-audio-002` - `packages/d2b-provider-audio-pipewire/src/argv.rs` (component template renderer) (adapt)
- [ ] T231 [US2] `ADR046-audio-004` - `packages/d2b-provider-audio-pipewire/src/mediator/enforcement.rs` (adapt)
- [ ] T232 [US2] `ADR046-audio-005` - `packages/d2b-provider-audio-pipewire/src/{resource_type (adapt)
- [ ] T233 [US2] `ADR046-audio-006` - `packages/d2b-provider-audio-pipewire/src/controller/audio_service.rs` (adapt)
- [ ] T234 [US2] `ADR046-audio-007` - `packages/d2b-provider-audio-pipewire/src/mediator/mod.rs` (create)
- [ ] T235 [US2] `ADR046-audio-008` - `nixos-modules/components/audio/v3-resource.nix` (replace)
- [ ] T236 [US2] `ADR046-audio-009` - `packages/d2b-provider-audio-pipewire/tests/minijail_contract.rs` (provider-local) (adapt)
- [ ] T237 [US2] `ADR046-audio-010` - `packages/d2b-provider-audio-pipewire/src/telemetry.rs` (adapt)
- [ ] T238 [US2] `ADR046-audio-011` - `packages/d2b-provider-audio-pipewire/src/guest_agent/mod.rs` (adapt)
- [ ] T239 [US2] `ADR046-audio-012` - `packages/d2b-provider-audio-pipewire/src/share_adapter.rs` (adapt)
- [ ] T240 [US2] `ADR046-audio-013` - `packages/d2b-provider-audio-pipewire/src/authority.rs` (speaker mixer + mic arbiter) (adapt)
- [ ] T241 [US2] `ADR046-audio-014` - `packages/d2b-provider-audio-pipewire/src/streams.rs` (adapt)

### Group `wi:ADR-046-provider-clipboard-wayland` (12 items)

- [ ] T242 [P] [US2] `ADR046-clipboard-001` - packages/d2b-provider-clipboard-wayland/ with src (create)
- [ ] T243 [US2] `ADR046-clipboard-002` - packages/d2b-provider-clipboard-wayland/src/clipd_host/ service binary modules such as service (adapt)
- [ ] T244 [US2] `ADR046-clipboard-003` - packages/d2b-provider-clipboard-wayland/src/controller/ and clipboard-controller binary (create)
- [ ] T245 [US2] `ADR046-clipboard-004` - packages/d2b-provider-clipboard-wayland/src/picker_session/ and picker-session binary (adapt)
- [ ] T246 [US2] `ADR046-clipboard-005` - packages/d2b-provider-clipboard-wayland service descriptors and generated Rust async ttrpc bindings (create)
- [ ] T247 [P] [US2] `ADR046-clipboard-006` - nixos-modules/providers/clipboard-wayland.nix and d2b.artifacts.clipboard-wayland catalog entry (replace)
- [ ] T248 [US2] `ADR046-clipboard-007` - packages/d2b-provider-clipboard-wayland/src/controller/rbac.rs or equivalent controller reconcile module (create)
- [ ] T249 [US2] `ADR046-clipboard-008` - packages/d2b-provider-clipboard-wayland/src/service/audit.rs and packages/d2b-provider-clipboard-wayland/src/service/metrics.rs (adapt)
- [ ] T250 [US2] `ADR046-clipboard-009` - packages/d2b-provider-clipboard-wayland/tests/ (extract)
- [ ] T251 [US2] `ADR046-clipboard-010` - packages/d2b-provider-clipboard-wayland/integration/ (create)
- [ ] T252 [US2] `ADR046-clipboard-011` - packages/d2b-contract-tests/tests/policy_clipboard.rs (adapt)
- [ ] T253 [US2] `ADR046-clipboard-012` - nixos-modules/default.nix (delete-after-cutover)

### Group `wi:ADR-046-provider-credential-entra` (1 items)

- [ ] T254 [US2] `ADR046-cred-entra-001` - `packages/d2b-provider-credential-entra/src/{lib.rs (adapt)

### Group `wi:ADR-046-provider-credential-managed-identity` (5 items)

- [ ] T255 [US2] `ADR046-cred-mi-001` - `packages/d2b-provider-credential-managed-identity/src/{lib.rs (adapt)
- [ ] T256 [US2] `ADR046-cred-mi-002` - packages/d2b-provider-credential-managed-identity/src/controller.rs (adapt)
- [ ] T257 [US2] `ADR046-cred-mi-003` - nixos-modules/options-resources.nix (replace)
- [ ] T258 [US2] `ADR046-cred-mi-004` - packages/d2b-provider-credential-managed-identity/src/{audit.rs (adapt)
- [ ] T259 [US2] `ADR046-mi-topology-001` - packages/d2b-provider-credential-managed-identity/src/{controller.rs (adapt)

### Group `wi:ADR-046-provider-credential-secret-service` (6 items)

- [ ] T260 [P] [US2] `ADR046-cred-ss-001` - packages/d2b-contracts/src/v3/credential.rs (adapt)
- [ ] T261 [P] [US2] `ADR046-cred-ss-002` - packages/d2b-contracts/proto/v3/credential.proto (create)
- [ ] T262 [US2] `ADR046-cred-ss-003` - `packages/d2b-provider-credential-secret-service/src/{lib.rs (adapt)
- [ ] T263 [P] [US2] `ADR046-cred-ss-004` - packages/d2b-provider-credential-<impl>/src/controller.rs (create)
- [ ] T264 [P] [US2] `ADR046-cred-ss-005` - nixos-modules/options-resources.nix (create)
- [ ] T265 [P] [US2] `ADR046-cred-ss-006` - packages/d2b-provider-credential-secret-service/src/{audit.rs (adapt)

### Group `wi:ADR-046-provider-device-gpu` (9 items)

- [ ] T266 [P] [US2] `ADR046-gpu-001` - `packages/d2b-provider-device-gpu/` with `src/` (extract)
- [ ] T267 [US2] `ADR046-gpu-002` - `packages/d2b-provider-device-gpu/src/{controller.rs (adapt)
- [ ] T268 [US2] `ADR046-gpu-003` - `packages/d2b-provider-device-gpu/src/probe.rs` (create)
- [ ] T269 [US2] `ADR046-gpu-004` - `packages/d2b-provider-device-gpu/src/arbitration.rs` (create)
- [ ] T270 [US2] `ADR046-gpu-005` - `packages/d2b-provider-device-gpu/src/worker_gpu.rs` (adapt)
- [ ] T271 [US2] `ADR046-gpu-006` - `packages/d2b-provider-device-gpu/src/worker_video.rs` (adapt)
- [ ] T272 [US2] `ADR046-gpu-007` - `nixos-modules/assertions.nix` (new GPU Device eval assertions) (adapt)
- [ ] T273 [US2] `ADR046-gpu-008` - `packages/d2b-provider-device-gpu/` component descriptor (create)
- [ ] T274 [US2] `ADR046-gpu-009` - `packages/d2b-provider-device-gpu/README.md` (create)

### Group `wi:ADR-046-provider-device-security-key` (35 items)

- [ ] T275 [US2] `ADR046-security-key-001` - Move to `packages/d2b-provider-device-security-key/src/session.rs` and `cid.rs` (adapt)
- [ ] T276 [US2] `ADR046-security-key-002` - Move to `packages/d2b-provider-device-security-key/src/relay.rs` (adapt)
- [ ] T277 [US2] `ADR046-security-key-003` - Adopt `main.rs` and `uhid.rs` as the v3 Process binary entry point (adapt)
- [ ] T278 [US2] `ADR046-security-key-004` - Preserve revalidation logic (adapt)
- [ ] T279 [US2] `ADR046-security-key-005` - Adapt to v3 Zone/ResourceRef identifiers (adapt)
- [ ] T280 [US2] `ADR046-security-key-006` - Move to `packages/d2b-provider-device-security-key/tests/` (adapt)
- [ ] T281 [US2] `ADR046-security-key-007` - Move to `packages/d2b-provider-device-security-key/tests/` (adapt)
- [ ] T282 [P] [US2] `ADR046-security-key-008` - New crate `packages/d2b-provider-device-security-key/` with `src/` (create)
- [ ] T283 [US2] `ADR046-security-key-009` - `packages/d2b-provider-device-security-key/src/controller.rs` (create)
- [ ] T284 [US2] `ADR046-security-key-010` - `packages/d2b-provider-device-security-key/src/relay.rs` (create)
- [ ] T285 [US2] `ADR046-security-key-011` - `packages/d2b-provider-device-security-key/src/session.rs` (create)
- [ ] T286 [US2] `ADR046-security-key-012` - `packages/d2b-provider-device-security-key/src/cid.rs` (create)
- [ ] T287 [US2] `ADR046-security-key-013` - `packages/d2b-provider-device-security-key/src/probe.rs` (create)
- [ ] T288 [US2] `ADR046-security-key-014` - `packages/d2b-provider-device-security-key/src/descriptor.rs` (create)
- [ ] T289 [US2] `ADR046-security-key-015` - `nixos-modules/minijail-profiles.nix` entries for relay and controller (create)
- [ ] T290 [US2] `ADR046-security-key-016` - Provider descriptor Process templates and owned CTAPHID `Endpoint` template for `Provider/device-security-key` (create)
- [ ] T291 [US2] `ADR046-security-key-017` - Signed Provider descriptor JSON for `Provider/device-security-key` in the provider package (create)
- [ ] T292 [US2] `ADR046-security-key-018` - v3 `SecurityKeyOpenDevice` broker op and Core LaunchTicket DeviceGrant resolution path (create)
- [ ] T293 [US2] `ADR046-security-key-019` - `nixos-modules/` resource compiler/eval assertions for physical Device (create)
- [ ] T294 [US2] `ADR046-security-key-020` - `nixos-modules/components/security-key-guest.nix` migration gate `d2b.securityKey._legacySystemdUnit` (create)
- [ ] T295 [US2] `ADR046-security-key-021` - Core `device-grant` audit and Provider controller Service/Binding ceremony lifecycle audit (create)
- [ ] T296 [US2] `ADR046-security-key-022` - Provider/controller bounded telemetry emitter and observability-otel handoff for security-key metrics (create)
- [ ] T297 [US2] `ADR046-security-key-023` - `packages/d2b-provider-device-security-key/README.md` (create)
- [ ] T298 [US2] `ADR046-security-key-024` - Authority/projection Service Endpoint and Binding private Endpoint resolution (create)
- [ ] T299 [US2] `ADR046-security-key-025` - `d2b-contracts` neutral `SecurityKeyEffectPort` trait/types (create)
- [ ] T300 [US2] `ADR046-security-key-026` - `packages/d2b-provider-device-security-key/src/{resource_type (create)
- [ ] T301 [US2] `ADR046-security-key-027` - Provider descriptor state declaration (create)
- [ ] T302 [US2] `ADR046-security-key-028` - `packages/d2b-provider-device-security-key/src/share_adapter.rs` (adapt)
- [ ] T303 [US2] `ADR046-security-key-029` - `packages/d2b-provider-device-security-key/src/{authority (adapt)
- [ ] T304 [US2] `ADR046-security-key-030` - Removed from daemon (delete-after-cutover)
- [ ] T305 [US2] `ADR046-security-key-031` - Removed from daemon startup (delete-after-cutover)
- [ ] T306 [US2] `ADR046-security-key-032` - Removed from guest Nix module (delete-after-cutover)
- [ ] T307 [US2] `ADR046-security-key-033` - Removed from `packages/d2b-contract-tests/tests/` (delete-after-cutover)
- [ ] T308 [US2] `ADR046-security-key-034` - Removed from `d2b-core/src/processes.rs` (delete-after-cutover)
- [ ] T309 [US2] `ADR046-security-key-035` - Removed from contracts and broker (delete-after-cutover)

### Group `wi:ADR-046-provider-device-tpm` (13 items)

- [ ] T310 [P] [US2] `ADR046-device-tpm-001` - packages/d2b-provider-device-tpm/{src/ (adapt)
- [ ] T311 [US2] `ADR046-device-tpm-002` - packages/d2b-provider-device-tpm/src/effect_port.rs (wrap)
- [ ] T312 [US2] `ADR046-device-tpm-003` - packages/d2b-provider-device-tpm/src/controller.rs (replace)
- [ ] T313 [US2] `ADR046-device-tpm-004` - packages/d2b-provider-device-tpm/src/resources.rs (replace)
- [ ] T314 [US2] `ADR046-device-tpm-005` - packages/d2b-provider-device-tpm/src/resources.rs (adapt)
- [ ] T315 [US2] `ADR046-device-tpm-006` - packages/d2b-provider-device-tpm/src/resources.rs (adapt)
- [ ] T316 [US2] `ADR046-device-tpm-007` - packages/d2b-provider-device-tpm/src/status.rs (create)
- [ ] T317 [US2] `ADR046-device-tpm-008` - packages/d2b-provider-device-tpm/src/{effect_port.rs (replace)
- [ ] T318 [US2] `ADR046-device-tpm-009` - packages/d2b-provider-device-tpm/tests/marker_fail_closed.rs (adapt)
- [ ] T319 [US2] `ADR046-device-tpm-010` - packages/d2b-provider-device-tpm/src/resources.rs (create)
- [ ] T320 [US2] `ADR046-device-tpm-011` - nixos-modules/options-resources.nix and Nix eval/golden tests for §17.1 Device JSON (replace)
- [ ] T321 [US2] `ADR046-device-tpm-012` - packages/d2b-provider-device-tpm/src/controller.rs (adapt)
- [ ] T322 [US2] `ADR046-device-tpm-013` - packages/d2bd/src/* (delete-after-cutover)

### Group `wi:ADR-046-provider-device-usbip` (9 items)

- [ ] T323 [P] [US2] `ADR046-usbip-001` - packages/d2b-contracts/src/usbip_effect_port.rs (create)
- [ ] T324 [US2] `ADR046-usbip-002` - packages/d2b-core/src/device_usbip_adapter.rs (adapt)
- [ ] T325 [US2] `ADR046-usbip-003` - packages/d2b-provider-device-usbip/ (create)
- [ ] T326 [US2] `ADR046-usbip-004` - packages/d2b-provider-device-usbip/src/{controller (adapt)
- [ ] T327 [US2] `ADR046-usbip-005` - packages/d2b-provider-device-usbip/src/reconcile.rs (adapt)
- [ ] T328 [US2] `ADR046-usbip-006` - packages/d2b-provider-device-usbip/src/status.rs (adapt)
- [ ] T329 [US2] `ADR046-usbip-007` - packages/d2b-provider-device-usbip/{src (adapt)
- [ ] T330 [US2] `ADR046-usbip-008` - nixos-modules/components/usbip.nix (adapt)
- [ ] T331 [US2] `ADR046-usbip-009` - packages/d2bd/src/ (delete-after-cutover)

### Group `wi:ADR-046-provider-display-wayland` (4 items)

- [ ] T332 [US2] `ADR046-display-001` - `packages/d2b-provider-display-wayland/src/` (adapt)
- [ ] T333 [US2] `ADR046-display-002` - Zone bundle emitter for `WaylandSession` / `WaylandPolicy` ResourceSpecs under `d2b.zones.<zone>.resources.*` (adapt)
- [ ] T334 [US2] `ADR046-display-003` - `packages/d2b-provider-display-wayland/src/audit.rs` (adapt)
- [ ] T335 [US2] `ADR046-display-004` - `packages/d2b-provider-display-wayland/integration/` (create)

### Group `wi:ADR-046-provider-network-local` (20 items)

- [ ] T336 [P] [US2] `ADR046-nl-001` - `d2b-contracts` trait plus `d2b-core` core adapter (create)
- [ ] T337 [US2] `ADR046-nl-002` - Broker wire contract and broker/core adapter operation table for `DeletePersistentTap` (adapt)
- [ ] T338 [P] [US2] `ADR046-nl-003` - `d2b-contracts` opaque byte-array newtypes (create)
- [ ] T339 [US2] `ADR046-nl-004` - Core LaunchTicket builder and dependency resolver that walks `Guest.ownerRef: Network/<name>` to resolved tap FDs. (create)
- [ ] T340 [P] [US2] `ADR046-nl-005` - Core adapter imports `d2b-host` modules (adapt)
- [ ] T341 [US2] `ADR046-nl-006` - `packages/d2b-provider-network-local/src/{controller.rs (adapt)
- [ ] T342 [P] [US2] `ADR046-nl-007` - `packages/d2b-provider-network-local/src/process_specs.rs` agent template plus agent service implementation in the net-VM artifact. (create)
- [ ] T343 [P] [US2] `ADR046-nl-008` - `packages/d2b-provider-network-local/src/config_volume.rs`. (adapt)
- [ ] T344 [P] [US2] `ADR046-nl-009` - `packages/d2b-provider-network-local/src/process_specs.rs`. (adapt)
- [ ] T345 [P] [US2] `ADR046-nl-010` - `net-vm-base` nixos-system artifact and artifact catalog entry `d2b.artifacts.net-vm-base`. (adapt)
- [ ] T346 [P] [US2] `ADR046-nl-011` - Nix module resource emission for `Provider/network-local` (adapt)
- [ ] T347 [P] [US2] `ADR046-nl-012` - Nix flake/resource schema checks for declared Networks and provider `validate.rs` parity. (adapt)
- [ ] T348 [P] [US2] `ADR046-nl-013` - `packages/d2b-provider-network-local/tests/schema_roundtrip.rs` (adapt)
- [ ] T349 [US2] `ADR046-nl-014` - `packages/d2b-provider-network-local/tests/controller_state.rs`. (create)
- [ ] T350 [P] [US2] `ADR046-nl-015` - `packages/d2b-provider-network-local/integration/host_fabric.rs` (adapt)
- [ ] T351 [P] [US2] `ADR046-nl-016` - Process templates for agent and dnsmasq plus sandbox/eval tests. (adapt)
- [ ] T352 [P] [US2] `ADR046-nl-017` - `packages/d2b-provider-network-local/README.md`. (create)
- [ ] T353 [P] [US2] `ADR046-nl-018` - Device-usbip EffectPort/adapter owns USBIP rules (adapt)
- [ ] T354 [P] [US2] `ADR046-nl-019` - Provider descriptor (create)
- [ ] T355 [P] [US2] `ADR046-nl-020` - Network schema/Provider descriptor (adapt)

### Group `wi:ADR-046-provider-notification-desktop` (6 items)

- [ ] T356 [P] [US2] `ADR046-notify-001` - `packages/d2b-provider-notification-desktop/src/{types (adapt)
- [ ] T357 [US2] `ADR046-notify-002` - `packages/d2b-provider-notification-desktop/src/stream_admission.rs` (adapt)
- [ ] T358 [US2] `ADR046-notify-003` - `packages/d2b-provider-notification-desktop/src/controller.rs` (create)
- [ ] T359 [US2] `ADR046-notify-004` - `packages/d2b-provider-notification-desktop/src/host_sink.rs` (adapt)
- [ ] T360 [US2] `ADR046-notify-005` - `packages/d2b-provider-notification-desktop/src/guest_source.rs` (create)
- [ ] T361 [US2] `ADR046-notify-006` - Nix: Zone resource authoring in `nixos-modules/` (adapt)

### Group `wi:ADR-046-provider-observability-otel` (6 items)

- [ ] T362 [US2] `ADR046-otel-001` - `packages/d2b-provider-observability-otel/src/{forwarder_bin (adapt)
- [ ] T363 [US2] `ADR046-otel-002` - `packages/d2b-provider-observability-otel/src/{collector_bin (adapt)
- [ ] T364 [US2] `ADR046-otel-003` - `packages/d2b-provider-observability-otel/src/nix/journald.nix` (adapt)
- [ ] T365 [US2] `ADR046-otel-004` - `packages/d2b-contract-tests/tests/policy_observability.rs` (updated) (adapt)
- [ ] T366 [US2] `ADR046-otel-005` - `packages/d2b-provider-observability-otel/src/share_adapter.rs` (adapt)
- [ ] T367 [US2] `ADR046-otel-006` - `packages/d2b-provider-observability-otel/src/{authority (adapt)

### Group `wi:ADR-046-provider-runtime-azure-container-apps` (7 items)

- [ ] T368 [US2] `ADR046-aca-001` - `packages/d2b-provider-runtime-azure-container-apps/src/controller.rs` (replace)
- [ ] T369 [US2] `ADR046-aca-002` - `packages/d2b-provider-runtime-azure-container-apps/src/deployment_service.rs` (adapt)
- [ ] T370 [US2] `ADR046-aca-003` - `packages/d2b-contracts/src/provider_effects/aca.rs` (shared `d2b-contracts` provider-effects module (adapt)
- [ ] T371 [US2] `ADR046-aca-004` - ACA sandbox-agent Endpoint/session controller (§§7 (replace)
- [ ] T372 [US2] `ADR046-aca-005` - `packages/d2b-provider-runtime-azure-container-apps/src/types.rs` (adapt)
- [ ] T373 [US2] `ADR046-aca-006` - `nixos-modules/` (generated Guest resource options) (replace)
- [ ] T374 [US2] `ADR046-aca-007` - `nixos-modules/` (gateway Guest declaration (create)

### Group `wi:ADR-046-provider-runtime-azure-virtual-machine` (9 items)

- [ ] T375 [P] [US2] `ADR046-azure-vm-001` - `src/{lib.rs (adapt)
- [ ] T376 [US2] `ADR046-azure-vm-002` - `src/effect/{mod.rs (adapt)
- [ ] T377 [US2] `ADR046-azure-vm-003` - `src/controller/{mod.rs (adapt)
- [ ] T378 [US2] `ADR046-azure-vm-004` - `src/controller/bootstrap.rs` (adapt)
- [ ] T379 [US2] `ADR046-azure-vm-005` - `src/credential.rs` (adapt)
- [ ] T380 [US2] `ADR046-azure-vm-006` - `src/controller/idempotency.rs` (adapt)
- [ ] T381 [US2] `ADR046-azure-vm-007` - `nixos-modules/` (Provider/Guest resource emitters) (adapt)
- [ ] T382 [US2] `ADR046-azure-vm-008` - `src/{telemetry.rs (adapt)
- [ ] T383 [P] [US2] `ADR046-azure-vm-009` - `tests/` (adapt)

### Group `wi:ADR-046-provider-runtime-cloud-hypervisor` (7 items)

- [ ] T384 [P] [US2] `ADR046-ch-001` - `packages/d2b-provider-runtime-cloud-hypervisor/src/controller.rs` (adapt)
- [ ] T385 [US2] `ADR046-ch-002` - `packages/d2b-provider-runtime-cloud-hypervisor/src/bootstrap_graph.rs` (replace)
- [ ] T386 [US2] `ADR046-ch-003` - `packages/d2b-provider-runtime-cloud-hypervisor/src/vmm_argv.rs` (adapt)
- [ ] T387 [US2] `ADR046-ch-004` - `packages/d2b-provider-runtime-cloud-hypervisor/nix/` (Nix emitter) (adapt)
- [ ] T388 [US2] `ADR046-ch-005` - `packages/d2b-provider-runtime-cloud-hypervisor/src/health.rs` (adapt)
- [ ] T389 [US2] `ADR046-ch-006` - `packages/d2b-provider-runtime-cloud-hypervisor/src/metrics.rs` (replace)
- [ ] T390 [US2] `ADR046-ch-007` - `packages/d2b-provider-runtime-cloud-hypervisor/src/state.rs` (replace)

### Group `wi:ADR-046-provider-runtime-qemu-media` (19 items)

- [ ] T391 [P] [US2] `ADR046-qemu-media-001` - packages/d2b-provider-runtime-qemu-media/{src/lib.rs (create)
- [ ] T392 [US2] `ADR046-qemu-media-002` - packages/d2b-provider-runtime-qemu-media/src/types/guest.rs (adapt)
- [ ] T393 [US2] `ADR046-qemu-media-003` - packages/d2b-provider-runtime-qemu-media/src/config.rs (adapt)
- [ ] T394 [US2] `ADR046-qemu-media-004` - packages/d2b-provider-runtime-qemu-media/src/{descriptor.rs (create)
- [ ] T395 [US2] `ADR046-qemu-media-005` - packages/d2b-provider-runtime-qemu-media/src/controller/volume.rs (adapt)
- [ ] T396 [US2] `ADR046-qemu-media-006` - packages/d2b-provider-runtime-qemu-media/src/controller/media_watch.rs (adapt)
- [ ] T397 [US2] `ADR046-qemu-media-007` - packages/d2b-provider-runtime-qemu-media/src/controller/device_watch.rs (create)
- [ ] T398 [US2] `ADR046-qemu-media-008` - packages/d2b-provider-runtime-qemu-media/src/controller/display.rs (create)
- [ ] T399 [US2] `ADR046-qemu-media-009` - packages/d2b-provider-runtime-qemu-media/src/controller/process_builder.rs (adapt)
- [ ] T400 [US2] `ADR046-qemu-media-010` - packages/d2b-provider-runtime-qemu-media/src/qmp/ (adapt)
- [ ] T401 [US2] `ADR046-qemu-media-011` - packages/d2b-provider-runtime-qemu-media/src/controller/hotplug.rs (adapt)
- [ ] T402 [US2] `ADR046-qemu-media-012` - packages/d2b-provider-runtime-qemu-media/src/controller/network.rs (create)
- [ ] T403 [US2] `ADR046-qemu-media-013` - packages/d2b-provider-runtime-qemu-media/src/controller/reconcile.rs (create)
- [ ] T404 [US2] `ADR046-qemu-media-014` - packages/d2b-provider-runtime-qemu-media/src/controller/status.rs (create)
- [ ] T405 [US2] `ADR046-qemu-media-015` - packages/d2b-provider-runtime-qemu-media/src/audit.rs (create)
- [ ] T406 [US2] `ADR046-qemu-media-016` - packages/d2b-provider-runtime-qemu-media/src/telemetry.rs (create)
- [ ] T407 [US2] `ADR046-qemu-media-017` - nixos-modules/options-guest-qemu-media.nix (adapt)
- [ ] T408 [US2] `ADR046-qemu-media-018` - packages/d2b-provider-runtime-qemu-media/tests/conformance_guest.rs (adapt)
- [ ] T409 [US2] `ADR046-qemu-media-019` - packages/d2b-provider-runtime-qemu-media/integration/ (create)

### Group `wi:ADR-046-provider-shell-terminal` (13 items)

- [ ] T410 [P] [US2] `ADR046-sterm-001` - `packages/d2b-provider-shell-terminal/src/resources/{pool (create)
- [ ] T411 [P] [US2] `ADR046-sterm-002` - `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-terminal-controller.rs` (create)
- [ ] T412 [P] [US2] `ADR046-sterm-003` - `packages/d2b-provider-shell-terminal/src/bin/d2b-shell-session-supervisor.rs` (adapt)
- [ ] T413 [P] [US2] `ADR046-sterm-004` - `packages/d2b-provider-shell-terminal/src/process_templates.rs` (replace)
- [ ] T414 [P] [US2] `ADR046-sterm-005` - `packages/d2b-provider-shell-terminal/src/service/open_session.rs` (create)
- [ ] T415 [P] [US2] `ADR046-sterm-006` - `packages/d2b-provider-shell-terminal/src/session/{pty (adapt)
- [ ] T416 [P] [US2] `ADR046-sterm-007` - `packages/d2b-provider-shell-terminal/src/session/adopt.rs` (adapt)
- [ ] T417 [P] [US2] `ADR046-sterm-008` - `packages/d2b-provider-shell-terminal/src/host_rules.rs` (replace)
- [ ] T418 [P] [US2] `ADR046-sterm-009` - `packages/d2b-provider-shell-terminal/src/guest_rules.rs` (replace)
- [ ] T419 [P] [US2] `ADR046-sterm-010` - `packages/d2b-provider-shell-terminal/src/authz.rs` (replace)
- [ ] T420 [P] [US2] `ADR046-sterm-011` - `packages/d2b-provider-shell-terminal/src/{audit (create)
- [ ] T421 [P] [US2] `ADR046-sterm-012` - `packages/d2b-provider-shell-terminal/src/migration.rs` (delete-after-cutover)
- [ ] T422 [P] [US2] `ADR046-sterm-013` - `packages/d2b-provider-shell-terminal/src/service/{controller (adapt)

### Group `wi:ADR-046-provider-system-core` (1 items)

- [ ] T423 [US2] `ADR046-system-core-001` - `packages/d2b-provider-system-core/src/manifest.rs` (adapt)

### Group `wi:ADR-046-provider-system-minijail` (6 items)

- [ ] T424 [US2] `ADR046-minijail-001` - `packages/d2b-provider-system-minijail/src/sandbox_compiler.rs` (adapt)
- [ ] T425 [US2] `ADR046-minijail-002` - Provider-side opaque request builder in `packages/d2b-provider-system-minijail/src/launch.rs` (adapt)
- [ ] T426 [US2] `ADR046-minijail-003` - Broker-side: `d2b-priv-broker` retains `SpawnRunner` and user-namespace pre-establishment (adapt)
- [ ] T427 [US2] `ADR046-minijail-004` - Broker-side parent wait/reap and typed terminal relay in `packages/d2b-priv-broker/src/` (adapt)
- [ ] T428 [US2] `ADR046-minijail-005` - `packages/d2b-provider-system-minijail/src/` - controller binary entry point (adapt)
- [ ] T429 [US2] `ADR046-minijail-006` - `nixos-modules/` - v3 Nix `Process`/`EphemeralProcess` resource authoring (adapt)

### Group `wi:ADR-046-provider-system-systemd` (3 items)

- [ ] T430 [US2] `ADR046-systemd-001` - `packages/d2b-provider-system-systemd/src/controller.rs` (async reconcile loop) (adapt)
- [ ] T431 [US2] `ADR046-systemd-002` - `nixos-modules/` (Provider ResourceSpec emission for `system-systemd`) (adapt)
- [ ] T432 [US2] `ADR046-systemd-003` - `packages/d2b-provider-system-systemd/tests/conformance.rs` (adapt)

### Group `wi:ADR-046-provider-transport-azure-relay` (7 items)

- [ ] T433 [P] [US2] `ADR046-transport-relay-001` - `packages/d2b-provider-transport-azure-relay/src/relay_transport.rs` (adapt)
- [ ] T434 [US2] `ADR046-transport-relay-002` - `packages/d2b-provider-transport-azure-relay/src/credential_client.rs` (create)
- [ ] T435 [US2] `ADR046-transport-relay-003` - `packages/d2b-provider-transport-azure-relay/src/reconnect.rs` (create)
- [ ] T436 [US2] `ADR046-transport-relay-004` - `packages/d2b-provider-transport-azure-relay/src/transport_settings.rs` (create)
- [ ] T437 [US2] `ADR046-transport-relay-005` - `packages/d2b-provider-transport-azure-relay/src/backpressure.rs` (adapt)
- [ ] T438 [US2] `ADR046-transport-relay-006` - `packages/d2b-provider-transport-azure-relay/src/{metrics.rs (create)
- [ ] T439 [P] [US2] `ADR046-transport-relay-007` - `packages/d2b-provider-transport-azure-relay/src/tests/integration/README` (create)

### Group `wi:ADR-046-provider-transport-unix` (11 items)

- [ ] T440 [US2] `ADR046-transport-unix-001` - `packages/d2b-provider-transport-unix/src/credit.rs` (imports `MAX_PACKET_ATTACHMENTS=32` (adapt)
- [ ] T441 [US2] `ADR046-transport-unix-002` - `packages/d2b-provider-transport-unix/src/{seqpacket (adapt)
- [ ] T442 [US2] `ADR046-transport-unix-003` - `packages/d2b-provider-transport-unix/src/{stream (adapt)
- [ ] T443 [US2] `ADR046-transport-unix-004` - `packages/d2b-provider-transport-unix/src/credit.rs` (adapt)
- [ ] T444 [US2] `ADR046-transport-unix-005` - `packages/d2b-provider-transport-unix/src/descriptor.rs` (adapt)
- [ ] T445 [US2] `ADR046-transport-unix-006` - `packages/d2b-provider-transport-unix/src/admission.rs` (adapt)
- [ ] T446 [US2] `ADR046-transport-unix-007` - `packages/d2b-provider-transport-unix/src/{portal (adapt)
- [ ] T447 [US2] `ADR046-transport-unix-008` - `packages/d2b-provider-transport-unix/` crate Cargo.toml binary target `d2b-transport-unix-service` (adapt)
- [ ] T448 [US2] `ADR046-transport-unix-009` - `docs/reference/schemas/v3/providers/transport-unix.transport-binding.json` (create)
- [ ] T449 [US2] `ADR046-transport-unix-010` - `packages/d2b-provider-transport-unix/src/{audit (create)
- [ ] T450 [US2] `ADR046-transport-unix-011` - `packages/d2b-provider-transport-unix/integration/` and `integration/README.md` (adapt)

### Group `wi:ADR-046-provider-transport-vsock` (7 items)

- [ ] T451 [US2] `ADR046-vsock-001` - `packages/d2b-provider-transport-vsock/src/effect_port.rs` (create)
- [ ] T452 [US2] `ADR046-vsock-002` - `packages/d2b-provider-transport-vsock/src/framing.rs` and `src/bridge.rs` (adapt)
- [ ] T453 [US2] `ADR046-vsock-003` - `packages/d2b-provider-transport-vsock/src/service.rs` (adapt)
- [ ] T454 [US2] `ADR046-vsock-004` - `d2b-core-controller` child Zone runtime `LiveVsockEffectPort` (adapt)
- [ ] T455 [P] [US2] `ADR046-vsock-005` - ProviderDeployment Volume creation/deletion path plus `packages/d2b-provider-transport-vsock/tests/state_volume.rs`. (create)
- [ ] T456 [US2] `ADR046-vsock-006` - `packages/d2b-provider-transport-vsock/integration/host_guest.rs` and `integration/no_fd_transfer.rs`. (create)
- [ ] T457 [P] [US2] `ADR046-vsock-007` - Remove legacy paths from `d2b-host` and `d2bd` (delete-after-cutover)

### Group `wi:ADR-046-provider-volume-local` (13 items)

- [ ] T458 [US2] `ADR046-vl-001` - `d2b-contracts/src/v3/volume_layout.rs` (LayoutEntry (adapt)
- [ ] T459 [US2] `ADR046-vl-002` - Full `packages/d2b-provider-volume-local/` scaffold per §Crate layout: `src/` (adapt)
- [ ] T460 [US2] `ADR046-vl-003` - `src/controller.rs` (adapt)
- [ ] T461 [US2] `ADR046-vl-004` - `src/store_view.rs` (adapt)
- [ ] T462 [US2] `ADR046-vl-005` - `src/swtpm_volume.rs` (adapt)
- [ ] T463 [US2] `ADR046-vl-006` - `src/source.rs` (block-image and tmpfs branches) (create)
- [ ] T464 [US2] `ADR046-vl-007` - `src/{migration (adapt)
- [ ] T465 [US2] `ADR046-vl-008` - `src/relocation.rs` (create)
- [ ] T466 [US2] `ADR046-vl-009` - `src/audit.rs` (adapt)
- [ ] T467 [US2] `ADR046-vl-010` - `nixos-modules/zone-resources.nix` (per §ADR046-pstate-010) (adapt)
- [ ] T468 [US2] `ADR046-vl-011` - `packages/xtask/src/provider_crate_policy.rs` (adapt)
- [ ] T469 [US2] `ADR046-vl-012` - `packages/d2b-host/src/volume_effect_adapter.rs` (or the equivalent host-runtime crate designated by the Zone broker owner) (adapt)
- [ ] T470 [US2] `ADR046-vl-013` - Zone core ProviderDeployment controller-start path (outside `d2b-provider-volume-local`) (create)

### Group `wi:ADR-046-provider-volume-virtiofs` (7 items)

- [ ] T471 [US2] `ADR046-vvfs-001` - `packages/d2b-provider-volume-virtiofs/src/virtiofsd_argv.rs` (adapt)
- [ ] T472 [US2] `ADR046-vvfs-002` - `packages/d2b-provider-volume-virtiofs/src/user_ns.rs` (conformance kit) (extract)
- [ ] T473 [US2] `ADR046-vvfs-003` - `packages/d2b-provider-volume-virtiofs/src/controller.rs` (adapt)
- [ ] T474 [US2] `ADR046-vvfs-004` - `packages/d2b-provider-volume-virtiofs/src/readiness.rs` (adapt)
- [ ] T475 [US2] `ADR046-vvfs-005` - `packages/d2b-provider-volume-virtiofs/src/controller.rs` (pre-launch prerequisite check) (adapt)
- [ ] T476 [US2] `ADR046-vvfs-006` - `nixos-modules/resources-volume.nix` (store-view and user Volume attachment emission) (adapt)
- [ ] T477 [US2] `ADR046-vvfs-export-001` - `packages/d2b-provider-volume-virtiofs/src/export.rs` (create)

### Group `wi:core-controller-coordination:w6` (1 items)

- [ ] T478 [US2] `ADR046-core-002` - `packages/d2b-core-controller/tests/system_core_coordination.rs` (adapt)

- [ ] T479 [US2] W6 GATE - run `/speckit.verify.run` and `/speckit.review.run` in parallel against this wave scope FIRST and clear their CRITICAL findings; then snapshot, import validation evidence, panel-request, panel-attest (10/10 unanimous), seal (every wave item Merged), merge-target, merge-eligibility. Also confirm for every item in this wave: reference docs landed with their behavior (FR-019), no change contradicts a decision in the register (FR-047), and every removal proof for a path retired in this wave passed (FR-023). From round 9, LOW/MEDIUM may be deferred to deferred-findings.md; CRITICAL/HIGH never (FR-051, FR-052). At wave close, review both registers and log this wave friction in friction-log.md (FR-053)
- [ ] T480 [US2] W6 CONVERGE + MERGE - merge every slice branch into the wave integration branch; run integration tests, panel, and CI against the converged tree only; open one PR against `v3`; merge after eligibility; rebase the next wave onto the updated `v3`; fold changelog fragments; then clean up in order: delete each worktree packages/target, remove worktrees, delete local branches, delete remote branches, nix-collect-garbage, and audit `git worktree list` plus `git branch -a` for residue

**Checkpoint**: W6 converged, panelled, sealed, merged to `v3`, rebased, and cleaned up. Successor entry criteria satisfied.

---

## Wave W7: Feasibility closure, reset and cutover, security, streamline, delivery contract

**Requirements**: see spec-coverage.md traceability tables | **Story**: US3 | **Work items**: 73 | **Parallel groups**: 5

- [ ] T481 [US3] W7 ENTRY - confirm every prior-wave work item is Merged, destinations uncontended, stack proposed against the exact parent commit, heavy-gate free, fast hermetic suite green

### Group `wi:ADR-046-feasibility-and-spikes` (10 items)

- [ ] T482 [US3] `ADR046-feasibility-002` - `proofs/process-fastlaunch-spike/` (adapt)
- [ ] T483 [P] [US3] `ADR046-feasibility-003` - `proofs/effectport-async-spike/` (adapt)
- [ ] T484 [P] [US3] `ADR046-feasibility-004` - `proofs/provider-packaging-spike/` (adapt)
- [ ] T485 [P] [US3] `ADR046-feasibility-005` - `proofs/bus-routing-noise-spike/` (adapt)
- [ ] T486 [P] [US3] `ADR046-feasibility-006` - `proofs/provider-state-export-spike/` (adapt)
- [ ] T487 [P] [US3] `ADR046-feasibility-007` - `proofs/process-provider-conformance-spike/` (adapt)
- [ ] T488 [P] [US3] `ADR046-feasibility-008` - `proofs/nix-authoring-spike/` (adapt)
- [ ] T489 [P] [US3] `ADR046-feasibility-009` - `proofs/cli-discovery-spike/` (adapt)
- [ ] T490 [US3] `ADR046-feasibility-010` - `proofs/e2e-composition-spike/` (adapt)
- [ ] T491 [US3] `ADR046-feasibility-011` - `proofs/test-runtime-budget-spike/` (adapt)

### Group `wi:ADR-046-reset-and-cutover` (11 items)

- [ ] T492 [P] [US3] `ADR046-reset-001` - `packages/d2b-cutover/src/{inventory (adapt)
- [ ] T493 [US3] `ADR046-reset-002` - `packages/d2b-cutover/src/{bundle_validate (adapt)
- [ ] T494 [US3] `ADR046-reset-003` - `packages/d2b-cutover/src/{consent (adapt)
- [ ] T495 [US3] `ADR046-reset-004` - `packages/d2b-cutover/src/adopt.rs` (adapt)
- [ ] T496 [US3] `ADR046-reset-005` - `packages/d2b-cutover/src/{store_bootstrap (create)
- [ ] T497 [US3] `ADR046-reset-006` - `packages/d2b-cutover/src/{zonelink_cutover (adapt)
- [ ] T498 [US3] `ADR046-reset-007` - `packages/d2b-cutover/src/{verify (adapt)
- [ ] T499 [US3] `ADR046-reset-008` - `packages/d2b-cutover/src/finalize.rs` (create)
- [ ] T500 [US3] `ADR046-reset-009` - `packages/d2b-cutover/src/{journal (adapt)
- [ ] T501 [US3] `ADR046-reset-010` - `packages/d2b-cutover/src/reset_scope.rs` (adapt)
- [ ] T502 [US3] `ADR046-reset-011` - `tests/integration/live/cutover-real-host.sh` (create)

### Group `wi:ADR-046-security-and-threat-model` (19 items)

- [ ] T503 [US3] `ADR046-security-001` - `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` (adapt)
- [ ] T504 [US3] `ADR046-security-002` - `packages/d2b-session/tests/noise_conformance.rs` (adapt)
- [ ] T505 [US3] `ADR046-security-003` - `packages/d2b-resource-store/tests/rbac_property.rs` (adapt)
- [ ] T506 [US3] `ADR046-security-004` - `packages/d2b-bus/fuzz/fuzz_targets/zonelink_frame.rs` (adapt)
- [ ] T507 [P] [US3] `ADR046-security-005` - `packages/xtask/src/effectport_boundary_check.rs` (adapt)
- [ ] T508 [US3] `ADR046-security-006` - `packages/d2b-provider-system-minijail/tests/launchticket_toctou.rs` (adapt)
- [ ] T509 [US3] `ADR046-security-007` - `packages/d2b-contract-tests/tests/quarantine_not_kill_matrix.rs` (adapt)
- [ ] T510 [US3] `ADR046-security-008` - `packages/d2b-provider-system-core/tests/no_isolation_propagation.rs` (adapt)
- [ ] T511 [US3] `ADR046-security-009` - `packages/d2b-provider-volume-local/tests/marker_tamper_fault_injection.rs` (adapt)
- [ ] T512 [US3] `ADR046-security-010` - `packages/d2b-contract-tests/tests/zero_secret_invariant.rs` (adapt)
- [ ] T513 [US3] `ADR046-security-011` - `packages/d2b-provider-{clipboard-wayland (adapt)
- [ ] T514 [US3] `ADR046-security-012` - `packages/d2b-audit/tests/privileged_fail_closed.rs` (adapt)
- [ ] T515 [US3] `ADR046-security-013` - `packages/d2b-bus/tests/dos_ceiling_fault_injection.rs` (adapt)
- [ ] T516 [US3] `ADR046-security-014` - `packages/d2b/src/commands/{doctor (adapt)
- [ ] T517 [US3] `ADR046-security-015` - `packages/d2b-core-controller/src/reset.rs` (adapt)
- [ ] T518 [P] [US3] `ADR046-security-016` - `tests/unit/gates/security-matrix-coverage.sh` (adapt)
- [ ] T519 [US3] `ADR046-security-017` - `tests/integration/containers/malicious-child-zone.rs` (adapt)
- [ ] T520 [P] [US3] `ADR046-security-018` - `docs/reference/security-manual-validation-checklist.md` (new reference doc (adapt)
- [ ] T521 [US3] `ADR046-security-019` - `packages/d2b-contract-tests/tests/minijail_process_ownership.rs` (adapt)

### Group `wi:ADR-046-streamline` (24 items)

- [ ] T522 [P] [US3] `ADR046-streamline-001` - `docs/specs/ADR-046-spec-set.json` (create)
- [ ] T523 [US3] `ADR046-streamline-002` - `docs/specs/schemas/*.schema.json` (Tier A: hand-authored-once canonical source checked into the tree (create)
- [ ] T524 [US3] `ADR046-streamline-003` - `packages/xtask/src/bin/spec_schema_check.rs` (create)
- [ ] T525 [US3] `ADR046-streamline-004` - `docs/specs/providers/TEMPLATE.md` (committed (create)
- [ ] T526 [US3] `ADR046-streamline-005` - `packages/d2b-contract-tests/tests/policy_spec_vocabulary.rs` (create)
- [ ] T527 [US3] `ADR046-streamline-006` - `packages/d2b-resource-store-redb/tests/provider_state_graph.rs` (or the eventual crate implementing Zone resource storage) (create)
- [ ] T528 [US3] `ADR046-streamline-007` - `packages/d2b-contract-tests/tests/policy_effectport_boundary.rs` (adapt)
- [ ] T529 [US3] `ADR046-streamline-008` - `packages/d2b-contract-tests/tests/policy_work_items.rs` (create)
- [ ] T530 [US3] `ADR046-streamline-009` - `docs/specs/ADR-046-provider-catalog.md` (generated (create)
- [ ] T531 [US3] `ADR046-streamline-010` - `tests/tools/reconcile-stale-base.sh` (reporting only) plus a documented `git town sync`/`git town` restack procedure this report feeds into (adapt)
- [ ] T532 [P] [US3] `ADR046-streamline-011` - `packages/xtask/src/bin/handoff_manifest.rs` (schema/validator only) (create)
- [ ] T533 [US3] `ADR046-streamline-012` - `tests/tools/import-task-db-consistency.sh` (create)
- [ ] T534 [US3] `ADR046-streamline-013` - `tests/tools/anti-serialization-report.sh` (adapt)
- [ ] T535 [P] [US3] `ADR046-streamline-014` - `tests/tools/run-layer.sh` extension (this repository already has `tests/tools/run-layer.sh` and `layer1-jobs.py` bounded-parallelism precedent) plus fake `EffectPort`/`ResourceClient` stub crates under `packages/d2b-provider-toolkit-fakes/` (adapt)
- [ ] T536 [US3] `ADR046-streamline-015` - Shared `packages/xtask` regeneration-conflict-detection helper consumed by every `gen-*`/`spec-registry` subcommand (adapt)
- [ ] T537 [US3] `ADR046-streamline-016` - `packages/d2b-contract-tests/tests/policy_no_leaked_decision_prefix.rs` (create)
- [ ] T538 [P] [US3] `ADR046-streamline-017` - `docs/specs/ADR-046-streamline-evidence-commands.md` (a follow-up artifact outside this task's file scope (adapt)
- [ ] T539 [US3] `ADR046-streamline-018` - `tests/tools/worktree-disk-report.sh` (adapt)
- [ ] T540 [US3] `ADR046-streamline-019` - `packages/xtask/src/bin/terminology_check.rs` (`cargo run -p xtask -- terminology-check`) (create)
- [ ] T541 [US3] `ADR046-streamline-020` - `packages/d2b-contract-tests/tests/policy_test_placement.rs` (create)
- [ ] T542 [US3] `ADR046-streamline-021` - `packages/d2b-contract-tests/tests/policy_test_determinism.rs` (create)
- [ ] T543 [US3] `ADR046-streamline-022` - `packages/xtask/src/test_runtime_ledger.rs` (shared with `ADR046-delivery-007`) (adapt)
- [ ] T544 [US3] `ADR046-streamline-023` - `packages/xtask/src/bin/legacy_test_retirement.rs` (`cargo run -p xtask -- legacy-test-retirement`) (adapt)
- [ ] T545 [US3] `ADR046-streamline-024` - `packages/xtask/src/bin/implementation_graph.rs` (`cargo run -p xtask -- implementation-graph`) (create)

### Group `wi:ADR-046-validation-and-delivery` (9 items)

- [ ] T546 [P] [US3] `ADR046-delivery-001` - `packages/xtask/src/heavy_gate.rs` (adapt)
- [ ] T547 [P] [US3] `ADR046-delivery-002` - `packages/xtask/src/delivery/snapshot.rs` (adapt)
- [ ] T548 [US3] `ADR046-delivery-003` - `packages/xtask/src/delivery/validate_import.rs` (adapt)
- [ ] T549 [US3] `ADR046-delivery-004` - `packages/xtask/src/gen_spec_set.rs` (adapt)
- [ ] T550 [US3] `ADR046-delivery-005` - `packages/xtask/src/delivery/panel.rs` (adapt)
- [ ] T551 [US3] `ADR046-delivery-006` - `packages/xtask/src/delivery/{seal (adapt)
- [ ] T552 [P] [US3] `ADR046-delivery-007` - `packages/xtask/src/test_runtime_ledger.rs` (adapt)
- [ ] T553 [US3] `ADR046-delivery-008` - `docs/specs/ADR-046-implementation-graph.json` (adapt)
- [ ] T554 [US3] `ADR046-delivery-009` - `packages/xtask/src/gen_spec_set.rs` (adapt)

- [ ] T580 [US3] **Implement the recovery-point attestation gate** (FR-043, SC-025): the cutover MUST refuse to execute any step past its rollback boundary until the operator has attested that a host recovery point exists, and every attestation MUST be recorded. **Tracked program-local, deliberately outside the work-item manifest** - FR-043 is stricter than `ADR-046-reset-and-cutover`, which permits proceeding without attestation. Consequence accepted: the W7 seal will NOT enforce this task, because seals check manifest items only. It is enforced by this task list and by the W7 MERGE review, not by the gate. Do not let a green W7 seal be read as evidence that FR-043 shipped

- [ ] T555 [US3] W7 GATE - run `/speckit.verify.run` and `/speckit.review.run` in parallel against this wave scope FIRST and clear their CRITICAL findings; then snapshot, import validation evidence, panel-request, panel-attest (10/10 unanimous), seal (every wave item Merged), merge-target, merge-eligibility. Also confirm for every item in this wave: reference docs landed with their behavior (FR-019), no change contradicts a decision in the register (FR-047), and every removal proof for a path retired in this wave passed (FR-023). From round 9, LOW/MEDIUM may be deferred to deferred-findings.md; CRITICAL/HIGH never (FR-051, FR-052). At wave close, review both registers and log this wave friction in friction-log.md (FR-053)
- [ ] T556 [US3] W7 CONVERGE + MERGE - merge every slice branch into the wave integration branch; run integration tests, panel, and CI against the converged tree only; open one PR against `v3`; merge after eligibility; rebase the next wave onto the updated `v3`; fold changelog fragments; then clean up in order: delete each worktree packages/target, remove worktrees, delete local branches, delete remote branches, nix-collect-garbage, and audit `git worktree list` plus `git branch -a` for residue

**Checkpoint**: W7 converged, panelled, sealed, merged to `v3`, rebased, and cleaned up. Successor entry criteria satisfied.

---

## Wave W8: Friction closure (terminal wave)

**Story**: US4 | **Work items**: recorded at W7 close | **Parallel groups**: TBD

W8 has **no spec members and no work items yet, by design**. Its contents are the delivery
friction accumulated across W0 through W7 - in the categories signoff, build, test, merge,
codegen, and disk - triaged at W7 close. Its destinations are `packages/xtask/`,
`tests/tools/`, `packages/d2b-contract-tests/tests/`, and `Makefile`.

It runs the same wave template unchanged, including exactly one binding ten-role panel.

- [ ] T557 [US4] W8 TRIAGE - collect and classify friction from every prior wave into the six categories; record the resulting work items in the manifest
- [ ] T558 [US4] W8 ENTRY - confirm W7 exit criteria hold
- [ ] T559 [US4] W8 IMPLEMENT - execute the triaged items (count known only after T557)
- [ ] T560 [US4] W8 GATE - snapshot, evidence, unanimous panel, seal, merge-eligibility
- [ ] T561 [US4] W8 MERGE - merge through pull requests; fold changelog; clean up

**Checkpoint**: the tree that actually ships now exists.

---

## Phase R: Release d2b 3.0

**Story**: US4. Evaluated against the **W8** candidate snapshot, not W7 - gating earlier would
release a candidate a later wave still modifies.

### The six release-gate conditions

- [ ] T562 [US4] Condition 1 - the five closing specs are Accepted and their work items' validation evidence is imported
- [ ] T563 [US4] Condition 2 - every DELETE and REPLACE row's removal proof passes **on the W8 candidate snapshot**, not merely when it was first established
- [ ] T564 [US4] Condition 3 - the complete validation matrix passes, including the manual hardware, live-host, and cloud tiers at least once with recorded external evidence, plus the reset and cutover scenarios
- [ ] T565 [US4] Condition 4 - unanimous ten-role panel on the W8 snapshot with zero recommendations; seal and merge-eligibility both pass
- [ ] T566 [US4] Condition 5 - `CHANGELOG.md` carries a new version header, summarized by version, with every internal wave, phase, and finding marker stripped
- [ ] T567 [US4] Condition 6 - every prior wave's cleanup is done; no dangling implementation worktrees or branches remain

### This program's additional release conditions

- [ ] T568 [US4] Confirm the companion inventory published at W5 (T577) is still accurate for the release candidate; a companion added or changed since W5 must be caught here
- [ ] T569 [US4] Verify each companion by exercising it against the release candidate on a live host - `d2b-toolkit`, `d2b-wlterm`, `d2b-wlcontrol`, `d2b-clip-picker`; `weezterm` consumes no d2b contract (FR-040, SC-024)
- [ ] T570 [US4] Confirm capability parity for every path whose migration disposition promised a successor (FR-041)
- [ ] T571 [US4] Publish the explicit retirement list with justifications, and name each retirement in the release notes (FR-042)
- [ ] T572 [US4] Confirm zero foundation surfaces remain deliberately unwired from production (SC-021)
- [ ] T573 [US4] Tag and publish d2b 3.0 from the `v3` lineage

**Checkpoint**: d2b 3.0 released.

---

## Pipelined wave execution

Panel review commonly runs **one to two times the coding duration**. Strictly serializing
review after implementation would idle the implementation capacity for more than half of every
cycle. Waves are therefore **pipelined**: the next wave starts coding while the current wave is
still in review, but nothing about the review gate is weakened.

### The pipeline

```text
W(N)   code ──> converge ──> integration tests ──> verify+review ──> panel (10 lanes) ──> seal 10/10 ──> merge to v3
                                    │                                                      │
                                    │ 5 of 10 panels back                                  │
                                    │ + integration green                                  │
                                    ▼                                                      ▼
W(N+1)                            code ──> converge ──> integration tests ──> verify+review ──> rebase on v3 ──> panel ──> seal ──> merge
```

### The four conditions (all required)

A wave may begin implementation early only when:

1. At least **5 of the 10** predecessor panel lanes have returned, **and**
2. the predecessor's **integration tests pass** on its converged tree, **and**
3. the successor issues **no panel request, no seal, and no merge** until the predecessor is
   sealed at 10/10 with zero recommendations and merged, **and**
4. the successor **rebases onto the updated `v3`** after that merge and **before** its own
   panel runs.

Condition 4 is what makes this safe: the successor's panel always binds to a snapshot that
already contains every finding the predecessor's panel produced. A panel never reviews a tree
built on unreviewed contracts.

### What is pipelined and what is not

| Activity | Pipelined? |
| --- | --- |
| Implementation coding | **Yes** - starts at 5 of 10 |
| Slice convergence onto the wave branch | **Yes** |
| Integration testing | **Yes**, subject to the 2-slot heavy-gate ceiling |
| Pre-panel verify + review gates | **Yes** - read-only, no heavy-gate slot |
| Panel request | **No** - strictly after predecessor seal + merge |
| Seal | **No** - strictly ordered |
| Merge to `v3` | **No** - strictly ordered |

### Accepted cost: rework

A predecessor finding that changes a contract may invalidate in-flight successor work. That
rework is **absorbed by the wave that started early** and is the explicit price of the
pipeline. It MUST NOT be used as an argument to weaken, shorten, or partially accept the
predecessor's panel (FR-050). If rework becomes chronic, the correct response is to start
later, not to review less.

### Governance status

This model required a constitution amendment. Principle VI was redefined in **constitution
2.0.0** to permit pipelined dispatch under exactly these four conditions. The ADR-046 delivery
spec and its tooling still enforce the stricter rule and must be amended before the pipeline
is executable - see T585-T587.

---

### Pre-panel verification gate

After a wave's slices converge and integration tests pass, but **before** any panel lane is
dispatched, the wave runs two read-only gates **in parallel**:

```text
slices converge ──> integration tests ──┬──> /speckit.verify.run  ──┐
                                        └──> /speckit.review.run ───┴──> panel (10 lanes) ──> seal
```

Both are strictly read-only. Neither modifies files; each emits a report.

| Gate | What it checks | Scope |
| --- | --- | --- |
| `/speckit.verify.run` | The wave's implementation against `spec.md`, `plan.md`, `tasks.md`, and the constitution. Constitution conflicts are automatically CRITICAL | The feature directory plus the converged wave tree |
| `/speckit.review.run` | Code quality across six specialized aspects: `code`, `comments`, `tests`, `errors`, `types`, `simplify` | The wave's diff only - see the scoping warning below |

### Run them in parallel, and parallelize inside review

Dispatch both in a single message. Within `/speckit.review.run`, its six aspect agents are
independent and read-only, so dispatch those in parallel too rather than sequentially. That is
up to seven concurrent read-only lanes; none takes a heavy-gate slot.

### Scoping warning: the review script targets the wrong branch by default

`.specify/extensions/review/scripts/bash/detect-changed-files.sh` resolves the default branch
from `git symbolic-ref refs/remotes/origin/HEAD`, which in this repository is **`main`**. The
ADR-046 integration lineage is **`v3`**, and `v3` never merges to `main`.

Left uncorrected, a wave review would diff the wave branch against `main` and treat the entire
v3 divergence - every prior wave, tens of thousands of lines - as this wave's changed files.
The review would be both unusably large and scoped to work that was already reviewed.

**Always pass an explicit scope.** The review command honours a caller-supplied file list or
retrieval instruction ahead of the script. Target the wave's own diff:

```bash
# The wave's changes only: integration branch vs its actual base
git diff --name-only $(git merge-base v3 adr046-w<N>-integrate)..adr046-w<N>-integrate

# When stacked on an unmerged predecessor, the base is that branch, not v3
git diff --name-only adr046-w<N-1>-integrate..adr046-w<N>-integrate
```

### Why this runs before the panel, not after

Both gates are cheap, automated, and read-only. The panel is ten reviewer lanes that commonly
cost one to two times the coding duration. Sending a wave to panel with defects these gates
would have caught spends the most expensive review capacity on the cheapest findings, and a
finding that arrives during panel forces a content change, which invalidates the snapshot and
every validation and panel record bound to it.

### Disposition of findings

- **verify.run CRITICAL** (including every constitution conflict) - fix before dispatching the
  panel. Do not carry it in.
- **review.run findings** - fix, or record in the deferred-findings register with a stated
  reason if genuinely LOW or MEDIUM. Note that the round-nine deferral rule governs *panel*
  rounds; these pre-panel gates have no round count and no deferral allowance of their own.
- Anything either gate raises that is actually a process problem rather than a code problem
  belongs in the friction log.

---

## Panel convergence and delivery memory

Panel rounds are bounded. From **round nine onward**, a reviewer may classify a LOW or MEDIUM
finding as **deferred** instead of blocking; CRITICAL and HIGH never become deferrable
(constitution 2.1.0, FR-051).

Deferral is a re-filing, not a dismissal. The finding moves **out of `recommendations`** into
[deferred-findings.md](./deferred-findings.md). That is what keeps `signoff` true if and only
if `recommendations` is empty - an invariant enforced in code at `panel.rs:277` in both
directions, so a deferred finding left in `recommendations` is rejected by `panel-attest`.
**No tooling change is required for this rule**, which is why it is shaped as a deferral
rather than as a relaxation of the sign-off check.

Two registers are maintained continuously, not written at the end:

| Register | Purpose | Feeds |
| --- | --- | --- |
| [deferred-findings.md](./deferred-findings.md) | Every deferred LOW/MEDIUM finding with its disposition | Next wave's planning; W8 triage |
| [friction-log.md](./friction-log.md) | What slowed delivery, in W8's six categories | W8 triage directly |

Neither may contain panel transcripts, command output, or attestation payloads - metadata only.

Per-wave obligations, folded into each wave's GATE task:

- Record any deferral at the moment it is made, not at wave close
- Review both registers at wave close; schedule or withdraw stale deferrals
- Log friction in the wave where it was felt, categorized on entry
- If the same friction category recurs across three waves, promote it to a task

---

## Branch, worktree, and convergence model

### Branch topology

Two tiers. Slice branches carry parallel subagent work; one integration branch per wave is the
convergence point and the only thing that becomes a PR against `v3`.

```text
v3
 └── adr046-w2-integrate          <- wave integration branch, PR target = v3
      ├── adr046-w2-primitives    <- slice branch, own worktree, own subagent
      └── adr046-w2-routing       <- slice branch, own worktree, own subagent

      └── adr046-w3-integrate     <- next wave STACKED on the unmerged w2 branch
           └── adr046-w3-provider
```

- **One worktree and branch per wave** for integration: `adr046-w<N>-integrate`.
- **One worktree and branch per slice**: `adr046-w<N>-<slice>`, cut from that wave's
  integration branch, never from `v3` directly.
- The next wave's integration branch is **stacked on the previous wave's integration branch
  while it is still unmerged**, and is re-pointed at `v3` once that merge lands.
- **All merges target `v3`.** Never `main`, never a local octopus merge, never a direct push.

### Convergence

Every parallel slice in a wave converges on that wave's integration branch **before** any of
the following runs. None of them are per-slice activities:

1. **Integration testing** - `make test-integration` and `make test-host-integration` against
   the converged tree, through the heavy-gate semaphore (2 concurrent lanes maximum)
2. **Panel review** - 10 read-only subagent lanes against the converged snapshot
3. **PR and CI** - one PR per wave integration branch, required checks green

Running any of these against an individual slice is wasted work: the snapshot the panel binds
to and the tree CI validates are the converged tree, not a slice.

### Wave lifecycle

```bash
# 1. Create the wave integration worktree, stacked on the prior wave if still unmerged
git worktree add -b adr046-w3-integrate ../d2b-w3 adr046-w2-integrate   # or v3 if w2 merged

# 2. Fan out one slice worktree per parallel group, cut from the integration branch
git worktree add -b adr046-w3-<slice> ../d2b-w3-<slice> adr046-w3-integrate

# 3. Subagents work in parallel, each confined to its own worktree and destinations

# 4. Converge: merge each slice into the wave integration branch
git -C ../d2b-w3 merge --no-ff adr046-w3-<slice>

# 5. Integration tests, panel, PR/CI against the converged branch

# 6. After the parent wave merges to v3, rebase this wave onto the updated v3
git -C ../d2b-w3 rebase v3
```

**Rebase invalidation**: a rebase changes history. Panel records may be reused **only** if the
byte-identical history proof passes (identical integrated content, identical generated
artifacts, identical dependency diff and repository set). Required CI **always** reruns on the
new history regardless. Treat a rebase that touches content as a re-snapshot.

### Cleanup (mandatory, not optional)

After a wave's PR merges, in this order:

```bash
# 1. Delete each slice worktree's real target dir FIRST or the removal reclaims nothing
rm -rf ../d2b-w3-<slice>/packages/target
git worktree remove ../d2b-w3-<slice>

# 2. Local branches
git branch -d adr046-w3-<slice>
git branch -d adr046-w3-integrate

# 3. Remote branches
git push origin --delete adr046-w3-<slice> adr046-w3-integrate

# 4. Store reclamation
nix-collect-garbage

# 5. Audit for orphans left by abandoned or superseded work
git worktree list
git branch -a | grep adr046-
```

A wave is not closed until `git worktree list` and `git branch -a` show no residue for it. The
release gate re-checks this: condition 6 requires the release to cut from a tree with no
dangling implementation worktrees or branches.

---

## Parallel subagent execution model

This plan is executed by a coding agent that dispatches **subagents**. Parallelism is the
default, not an optimization: the delivery contract makes launching every ready, file-disjoint
slice in the same coordination cycle a **positive obligation**, and a launch count below the
ready count without a recorded blocker fails wave entry criteria.

### Dispatch rule

When several slices are ready and file-disjoint, dispatch them **in a single message with
multiple subagent calls**. Sequential dispatch of independent slices is a process failure to
correct, not a scheduling preference.

### Per-wave implementation fan-out

One write-capable subagent per parallel group, each in its **own git worktree** so no two
agents share a working tree or a `packages/target/`:

| Wave | Groups | Concurrent implementation subagents |
| --- | --- | --- |
| W2 | 2 | **2** - zero file-overlap edges; both start together |
| W3 | 1 | **1** - strictly serial by design |
| W4 | 6 | **6** |
| W5 | 12 | **12**, minus the serial store chain (`store-004` -> `store-002` -> `reconcile-003`) |
| W6 | 28 | **up to 27** - each Provider's hermetic suite compiles without any other Provider existing |
| W7 | 5 | **5** |

Worktree setup per slice (cut from the wave integration branch, never from `v3`):

```bash
git worktree add -b adr046-w<N>-<slice> ../d2b-w<N>-<slice> adr046-w<N>-integrate
```

Before removing a worktree, delete its real `packages/target/` or the removal reclaims
nothing. Compiled-output dedup across worktrees comes from `sccache`, not a shared target dir.

### Panel fan-out

Panels run as **10 read-only subagent lanes on `gemini-3.1-pro-preview`**, one per roster role
(`software`, `test`, `nixos`, `networking`, `security`, `rust`, `product`, `docs`,
`observability`, `kernel`), dispatched together in one message.

- Lanes are **read-only by contract**. They inspect the diff, the plan, and the integrator's
  supplied validation evidence. They MUST NOT run tests, builds, evals, or long validations
  unless the integrator explicitly asks a specific lane to.
- Because they are read-only they take **no heavy-gate slot**, so all 10 run concurrently
  without contending for the semaphore.
- Each lane's verdict maps one-to-one onto a `panel-attest` record. `signoff` is true **iff**
  `recommendations` is empty.
- Every lane must run on the pinned model. A lane that silently falls back to another model
  produces a record `panel-attest` rejects, which blocks the seal.

### Hard concurrency ceilings

| Resource | Ceiling | Consequence of exceeding |
| --- | --- | --- |
| Heavy lanes (Layer 2, hardware, live, perf) | **2 per uid**, enforced by the OFD-locked semaphore | Blocks up to 30 minutes, then fails closed. Never add a second lock or a retry loop |
| Read-only panel lanes | No semaphore limit | None |
| Layer-1 subagent builds | Bounded by disk, not by lock | Each worktree carries its own multi-GiB `packages/target/` |
| Free disk under the repo root | **10 GiB minimum**, fail-closed preflight | Wave fails at the preflight guard |

The practical consequence: **implementation fan-out can be wide, heavy validation cannot.**
Dispatch 27 Provider subagents freely, but serialize their `make test-integration` and
`make test-host-integration` runs through the gate.

### What each subagent needs in its prompt

1. Its work-item ids, retrieved verbatim from the manifest - `detailedDesign`, `validation`,
   `destination`, `removalProof` are the task
2. Its exact destination paths, with an instruction to write nowhere else
3. Its worktree path and branch
4. The reminder that contended files are integrator-owned and that `CHANGELOG.md` is never
   edited by a slice - each writes one `changelog.d/<branch>.md` fragment instead
5. The commit-tag form: `( ADR046-W<n> )`, or `( ADR046-W<n>fu<m> <S><n> )` for a finding fix

### Integrator-only work (never delegated to a slice subagent)

Contended files, the spec-set and work-item manifests (last commit of each wave), the
changelog fold, the wave snapshot/panel/seal/merge sequence, and worktree cleanup.

---

## Dependencies and execution strategy

### Between waves - strictly sequential

```text
Phase 0 -> W2 -> W3 -> W4 -> W5 -> W6 -> W7 -> W8 -> Release

Pipelined: W(N+1) coding starts at 5 of 10 W(N) panels + green integration.
W(N+1) panel/seal/merge remain strictly after W(N) seal + merge + rebase.
```

Panel, seal, and merge are strictly ordered. Only implementation start is pipelined (FR-048,
FR-049, FR-050; constitution 2.0.0 Principle VI).

### Within a wave - maximize parallelism

Parallel groups are file-disjoint by construction. Launch every ready group in the same
coordination cycle; a launch count below the ready count without a recorded blocker **fails
wave entry criteria**.

| Wave | Groups | Parallelism note |
| --- | --- | --- |
| W2 | 2 | Zero file-overlap edges; both groups start together. The 3 `primitives` items have no intra-wave dependency at all |
| W3 | 1 | Strictly serial by design; every Provider dossier depends on it |
| W4 | 6 | Five parallel specs |
| W5 | 12 | Largest coordination load; the store chain is serial inside it |
| W6 | 28 | 27 crates in five families; each Provider's hermetic suite compiles without any other Provider existing |
| W7 | 5 | Closing group |

### The 14 file-overlap ordering constraints

These are the only edges that constrain shared files. Honor them as strict ordering:

- W5: `ADR046-device-006` -> `ADR046-nix-014` -> `ADR046-cli-011` -> `ADR046-nix-019` -> `ADR046-nix-031`
- W6: `ADR046-gpu-007` -> `ADR046-transport-unix-009` -> `ADR046-qemu-media-017` -> `ADR046-usbip-008`
- `ADR046-core-001` precedes `ADR046-device-007`, `-exec-013`, `-exec-015`, `-network-008`, `-telem-011`, `-zone-control-016`, `-zone-control-021`

### Contended files - integrator only

`packages/d2b-contracts/src/v3/volume.rs`; the `packages/Cargo.toml` member list; the
`flake.nix` output list; `nixos-modules/index.nix` and `default.nix`;
`packages/d2b-contract-tests/tests/workspace_policy.rs`; and the spec-set and work-item
manifests (last commit of each wave).

`CHANGELOG.md` is different: **no slice edits it**. Each writes one `changelog.d/<branch>.md`
fragment; the integrator folds them at wave close.

### Heavy lanes

Every Layer-2, hardware, live, and perf command runs through the two-slot semaphore. Never
invoke an internal `heavy-lane-*` target directly.

---

## Implementation strategy

### Critical path

W3 is a single serial spec that gates all 27 dossiers, so it is the narrowest point in the
program. W5 carries the highest risk (RK-1, the corrected store engine), and W6 carries the
highest volume (257 items). T007 exists specifically so W5's risk is retired early rather than
discovered on the critical path.

### Incremental value

Each user story is independently demonstrable:

- **After W5** - US1 is complete. An operator can declare a Zone and watch resources become
  real. This is the first point at which the program has produced operator-visible value.
- **After W6** - US2 is complete. Capabilities arrive declaratively through Providers.
- **After W7** - US3 is complete. An existing host can move onto 3.0.
- **After W8 plus Release** - US4 is complete.

Note that no intermediate release ships (FR-045). These are internal checkpoints, not
deliverables.

### Task count

587 tasks: 17 pre-wave hygiene (4 panel-model migration, 3 pipelined-wave migration), 531 work items,
18 wave entry/gate/merge tasks for W2-W7, 5 for the terminal wave, 4 added at W5/W7 by the
analysis remediation, and 12 for the release.
The 531 work-item tasks correspond exactly to the 531
`Planned` items in the manifest - one task each, no more and no fewer. Task ids added after
the initial generation continue from T574 rather than renumbering the original 573.
