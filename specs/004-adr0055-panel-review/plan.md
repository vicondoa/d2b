# Implementation Plan: Pragmatic ADR 0055 Panel Review

**Branch**: `spec004-panel-review-pragmatic` | **Date**: 2026-08-08 | **Spec**: [spec.md](./spec.md)

**Input**: Accepted ADR 0055 and the feature specification in
`specs/004-adr0055-panel-review/spec.md`.

## Summary

Implement ADR 0055 as one atomic Track B cutover of the standard Copilot panel
skill. Keep one comprehensive discovery, let the orchestrator assign stable
`R1`...`Rn` identifiers and merge one ledger, batch all implementation
responses and self-verification, then run scoped verification. Verification
admits only introduced regressions, unsafe late issues, and previously missed
`BLOCKER` or `MAJOR` issues.

Use one versioned selection table to choose a deterministic roster, including
optional `build`, with Rust as a `software` profile. For every reviewed
candidate state, render one versioned selection artifact bound to the
candidate and lifecycle; xtask `panel-request` and `make-records.mjs` consume
and validate that same artifact. Reselect over the full candidate and each fix
delta and only widen the lifecycle roster. Generate the discovery, ledger,
response, and per-seat verification artifacts automatically; fail on
conflicting regeneration. Automatically import complete or partial legacy
rounds, preserve raw findings and attribution, and keep metrics informational.
Keep the existing schema-version-2 panel request, record, attestation, and seal
shapes unchanged. The request's existing `roles` and `record_files` fields
carry the selected roster, and existing ten-seat artifacts remain readable.

This is contributor tooling only. It adds no service, daemon, broker surface,
socket, operating-system principal, authorization protocol, cryptographic
receipt, privileged audit path, crate, Gas City implementation, or exhaustive
crash/process machinery.

## Spec Corrections

| Source drift | Canonical state observed before implementation | Treatment |
| --- | --- | --- |
| The feature describes the accepted lifecycle as the target behavior. | `stage-diffs.sh`, `make-records.mjs`, the panel agents, and xtask still implement repeated fixed ten-seat rounds. | Preserve that behavior until every replacement part lands in `spec004w1`; do not ship a half-cutover. |
| Current contributor docs and Constitution Principle VI name a fixed ten-seat roster. | Passing `PANEL_ROLES` validation and those docs agree today. | Update code and every binding contributor document together to the ADR 0055 selected-roster contract. |
| Current delivery records require exactly the legacy ten roles. | `packages/xtask/src/delivery/panel.rs` and `make-records.mjs` each carry a separate fixed roster. No shared candidate-bound selection artifact exists. | Generate one lifecycle-selection artifact per candidate state, require both producers to consume it, populate the request's existing ordered `roles` and `record_files`, and validate records against that request-bound roster. |
| Every delivery artifact currently uses the workspace-wide schema version `2` and has no selection or lifecycle fields. | Existing request, record, attestation, and seal shapes already carry the roster and candidate data needed for compatibility. | Keep those serialized shapes and `DELIVERY_SCHEMA_VERSION` unchanged. Version only the lifecycle-selection artifact, retain `rust` in the allowed role domain for existing data, and exclude it from current selection. |
| The checked prompt corpus enforces an exact 32-file shape with thirteen agent files, and ADR 0053's prompt-source specification describes a twelve-role pool with no build seat. | `prompt-corpus.mjs`, its binding tests, and `docs/adr/specs/0053-panel-prompt-sources.md` agree with the current pool. | Update the prompt-source build guidance and pin the prompt-corpus shape and tests to 35 files with sixteen agent files in the same atomic cutover as the build agent. |

## Technical Context

**Language/Version**: Existing Node.js ES modules, Bash, repository-pinned Rust,
JSON, and Markdown.

**Primary Dependencies**: Node.js standard library, Git, existing Copilot
skills and read-only agents, and existing xtask dependencies (`serde`,
`serde_json`, and `sha2`). No new package or crate dependency.

**Storage**: Versioned JSON and Markdown in `.scratch/panel/<lifecycle>/`,
including immutable selections under
`selections/<candidate-id>/<snapshot-sha256>.json`, plus the existing external
candidate-addressed delivery state. No database or privileged storage.

**Testing**: Plain Node.js behavior tests through `make test-lint`, xtask unit
tests, focused xtask clippy and formatting, Tier 0, changelog validation, and
policy only for governed documentation changes.

**Target Platform**: Linux contributor worktrees using GitHub Copilot CLI; no
d2b runtime or guest surface.

**Project Type**: Contributor workflow, prompt, script, and existing xtask CLI
integration.

**Performance Goals**: Exactly one discovery dispatch per lifecycle, linear
processing in selected seats and findings, and byte-identical output for
identical inputs.

**Constraints**: Fail closed on selection candidate, selection-schema,
selection-table-version, or ordered-roster mismatch; missing seat results,
mappings, or ledger responses;
incomplete required justification, evidence, factual verification, or
acceptance; conflicting bytes; roster narrowing; malformed legacy data;
unrelated fix scope; or incomplete merge blockers. Cutover is one PR with no
wave seal.

**Scale/Scope**: Thirteen-seat pool, code/configuration floor of ten,
documentation-only floor of eight, existing ten-seat legacy rounds, and
lifecycle-local stable issue identifiers.

## Constitution Check

**Track: B.** This is a contained refactor of contributor-only Copilot, script,
prompt, documentation, and xtask delivery-policy surfaces. It does not change a
product wire/schema, broker operation, trust boundary, persistent root surface,
or critical-subsystems index entry. The selected-roster data is review process
state, not a d2b runtime protocol. Track B therefore uses one atomic wave, one
finished-diff panel lifecycle, one PR, and no seal.

- **I and II - control plane and privilege**: Pass. No runtime unit, daemon,
  socket, principal, broker operation, capability, authorization, or audit
  surface is introduced.
- **III - isolation**: Pass. No VM or host isolation path is touched.
- **IV - compatibility**: Pass. Panel requests, records, attestations, and
  seals retain their existing schema-version-2 serialized shapes. Existing
  ten-seat data, including `rust`, remains readable through the same DTOs,
  while current selection simply does not choose `rust`.
- **V - test discipline**: Pass. Behavior is covered by existing Layer 1
  Node.js and Rust unit surfaces; no new top-level gate or integration tier is
  added.
- **VI - panel gating**: Pass. `spec004w1` is one phase. Plan review uses the
  current passing panel before dispatch; work review uses one discovery and
  scoped verification with unanimous sign-off from the selected lifecycle
  roster.
- **VII - traceability**: Pass. `spec004w1` appears only in this plan and
  commit/PR process metadata, never shipped source, contributor prose, or the
  changelog fragment.

The check remains passed after design: both implementation slices are
serialized, the cutover is atomic, and the expected binding docs are updated
with the code. There is no constitutional exception.

For this feature's implementation panel, only `CRITICAL` and `HIGH` defects
that make the accepted ADR lifecycle unsafe or incorrect are merge-blocking
recommendations. Lower-severity, optional, or scope-expanding feedback belongs
in the summary and does not enter the fix ledger. The operator explicitly
rejected scope-expanding feedback. Current verdict JSON continues to spell
those values `critical` and `high`.

## Project Structure

### Documentation (this feature)

```text
specs/004-adr0055-panel-review/
├── checklists/
│   └── requirements.md
├── contracts/
│   └── panel-artifacts.md
├── data-model.md
├── plan.md
├── quickstart.md
├── research.md
├── spec.md
└── tasks.md
```

The research, data model, contract, and quickstart artifacts record the
accepted design and validation contract. The existing `.specify/feature.json`
active-feature pointer is also a declared feature artifact for the branch.

### Source Code (repository root)

```text
.github/
├── agents/
│   ├── panel-{software,test,product,docs,security,observability}.agent.md
│   ├── panel-{nixos,networking,kernel}.agent.md
│   ├── panel-{simplicity,reliability,agentic,build}.agent.md
│   ├── panel-rust.agent.md
│   └── d2b-integrator.agent.md
└── skills/
    ├── d2b-panel-round/
    │   ├── SKILL.md
    │   ├── selection-table.json
    │   └── scripts/{stage-diffs.sh,make-records.mjs,panel-lifecycle.mjs}
    ├── d2b-autopilot/SKILL.md
    └── d2b-wave-delivery/SKILL.md

scripts/copilot/
├── check-bindings.mjs
├── prompt-corpus.mjs
├── prompt-corpus-manifest.json
├── test-check-bindings.mjs
├── test-stage-diffs.mjs
├── test-make-records.mjs
└── test-panel-lifecycle.mjs

packages/xtask/src/delivery/
├── model.rs
├── panel.rs
├── seal.rs
├── evidence.rs
├── command.rs
└── mod.rs

tests/test-lint.sh
AGENTS.md
.specify/memory/constitution.md
docs/contributing/{README.md,panel-review.md,copilot-agents.md}
docs/adr/specs/0053-panel-prompt-sources.md
docs/specs/ADR-046-validation-and-delivery.md
changelog.d/adr055-panel-review.md
```

**Structure Decision**: Keep the behavior in the existing panel skill and its
scripts. Add only one focused JavaScript lifecycle helper and the normative
selection table. Retire the separate `panel-rust` agent only after legacy Rust
attribution maps to the Rust profile on `software`. Extend the existing xtask
crate rather than adding a crate. Exercise compatibility through existing test
helpers rather than creating a parallel delivery schema or fixture family. The
ownership set below is closed for this wave; an undeclared changed path fails
the final allowlist instead of being admitted by an implementation-time
adjustment.

## Wave Graph

```text
spec004w1.lifecycle-selection
    -> spec004w1.delivery-docs
    -> focused validation
    -> selected-roster work review
    -> one Track B PR
```

There is one independently reviewable and mergeable wave: `spec004w1`.
The two slices are serialized because delivery and documentation consume the
selection and lifecycle contracts from the first slice. No integrator prep
commit is needed, and no second wave or PR may expose an intermediate format.

## Wave `spec004w1` - Atomic Panel Lifecycle Cutover

### Deliverable

The standard Copilot panel performs deterministic selection, one comprehensive
discovery, one merged ledger, batched responses and self-verification, and
scoped verification. Generated artifacts, complete and partial legacy import,
monotonic roster widening, informational metrics, and selected-roster xtask
delivery validation work together at merge.

### Slice 1 - Lifecycle and Selection Tooling

**Owned files**

- `.github/skills/d2b-panel-round/SKILL.md`
- `.github/skills/d2b-panel-round/selection-table.json` (new)
- `.github/skills/d2b-panel-round/scripts/stage-diffs.sh`
- `.github/skills/d2b-panel-round/scripts/make-records.mjs`
- `.github/skills/d2b-panel-round/scripts/panel-lifecycle.mjs` (new)
- Retained panel agents:
  `.github/agents/panel-{software,test,product,docs,security,observability,nixos,networking,kernel}.agent.md`
- New panel agents:
  `.github/agents/panel-{simplicity,reliability,agentic,build}.agent.md`
- Retired current-only agent: `.github/agents/panel-rust.agent.md`
- `scripts/copilot/check-bindings.mjs`
- `scripts/copilot/test-check-bindings.mjs`
- `scripts/copilot/test-stage-diffs.mjs`
- `scripts/copilot/test-make-records.mjs`
- `scripts/copilot/test-panel-lifecycle.mjs` (new)
- `tests/test-lint.sh`

**Work**

1. Make selection-table version 2 authoritative for the thirteen-seat pool,
   floors, fill order, triggers, focus, and profiles. Select optional `build`
   for actual build-contract surfaces but not citation-only prose. Bind Rust
   review depth as a `software` profile. Generate or byte-check human guidance
   from this table so prompts cannot drift from selection. Update the
   `test-check-bindings.mjs` corpus-shape cases from 32 files and thirteen
   agents to 35 files and sixteen agents for the build agent and retired Rust
   agent.
2. Render selection schema version 1 at
   `.scratch/panel/<lifecycle>/selections/<candidate-id>/<snapshot-sha256>.json`.
   Bind lifecycle id, phase, program, wave, the candidate digest triple,
   selection-table version, classification, profiles, and ordered lifecycle
   roster. `stage-diffs.sh` records the lifecycle id, and the helper passes the
   same selection path to xtask `panel-request` and `make-records.mjs`.
   Lifecycle metadata stays in the version-1 lifecycle artifact; neither
   consumer adds it to delivery artifacts.
3. Generate discovery requests, merged ledgers, response templates, validation
   and self-review sections, and per-seat verification requests. The
   orchestrator assigns deterministic `R` identifiers after deduplication and
   validates complete source-to-ledger mappings. Require an explicit complete
   result from every selected seat, accepting `findings: []` but refusing a
   missing seat result.
4. Require exactly one response for every ledger issue. Preserve the ADR 0055
   disposition set exactly: `Fixed`, `Intentionally rejected`, `Deferred`,
   `Withdrawn`, and `Invalid`. Validate disposition-specific justification,
   change evidence, and verified factual status. A BLOCKER approves only as
   Fixed, Invalid, or Withdrawn, with factual verification for the latter two.
   A MAJOR approves when Fixed or when its non-Fixed response records acceptance
   by the repository maintainer or merge owner. Deferred cannot approve a
   BLOCKER or an unaccepted MAJOR.
5. Reselect over the full candidate and every fix delta, union the result with
   the prior lifecycle roster, and reject narrowing or unrelated scope
   expansion with an actionable new-lifecycle remedy.
6. Admit late verification issues only under ADR 0055 rules. Keep MINOR/NIT
   history in the ledger, not in blocking recommendations.
7. Import complete legacy rounds as discovery and partial rounds as preserved
   sources followed by the lifecycle's one current discovery. Dispatch by
   version before parsing, identify a source by record digest, seat, and
   recommendation ordinal, preserve raw bytes and attribution, and map legacy
   `rust` responsibility to the `software` Rust profile. Map exact bracketed
   legacy prefixes `[critical]`, `[high]`, `[medium]`, and `[low]` to
   `BLOCKER`, `MAJOR`, `MINOR`, and `NIT` with ASCII case folding; classify
   every other spelling as migration-assigned `MAJOR`. Preserve imported
   obligations in the roster union and make repeated import byte-stable.
8. Calculate initial, late, severity, review-iteration,
   implementation-iteration, and fixed-per-iteration metrics. Never consult a
   metric when deciding approval.
9. Make `make-records.mjs` require the lifecycle-selection path. Validate its
   schema version, selection-table version, candidate identity, and ordered
   roster against `candidate.json`, verdicts, observed bindings, and emitted
   records. Emit the existing schema-version-2 `PanelRecord` shape for exactly
   those roles. Preserve model and effort checks, candidate binding, distinct
   run provenance, and `signoff == recommendations.is_empty()`.

### Slice 2 - Delivery and Documentation Integration

**Owned files**

- `packages/xtask/src/delivery/{model,panel,seal,evidence,command,mod}.rs`
- `.github/agents/d2b-integrator.agent.md`
- `.github/skills/d2b-autopilot/SKILL.md`
- `.github/skills/d2b-wave-delivery/SKILL.md`
- `AGENTS.md`
- `.specify/memory/constitution.md`
- `docs/contributing/README.md`
- `docs/contributing/panel-review.md`
- `docs/contributing/copilot-agents.md`
- `docs/adr/specs/0053-panel-prompt-sources.md`
- `docs/specs/ADR-046-validation-and-delivery.md`
- `changelog.d/adr055-panel-review.md`
- `scripts/copilot/prompt-corpus.mjs`
- `scripts/copilot/prompt-corpus-manifest.json`

**Work**

1. Parse the version-1 lifecycle-selection artifact at xtask
   `panel-request --selection`, validate its candidate identity,
   selection-schema version, selection-table version, and ordered roster, and
   populate the existing schema-version-2 request's `roles` and `record_files`.
   Do not add a panel envelope, alternate DTO, or delivery field.
2. Keep the existing schema-version-2 `PanelRequest`, `PanelRecord`,
   `PanelAttestation`, and `SealRecord` serialized shapes. Retain
   `PanelRole::Rust` in the allowed role domain so existing ten-seat requests
   and records parse naturally, while the current selection table excludes it.
3. Make `panel-attest` validate exactly the ordered roles and record files
   already stored in the request. Focused tests use existing helpers to cover
   variable rosters, missing and extra records, mismatched selections, and a
   ten-seat request with `rust`.
4. Preserve model and effort checks, candidate binding, distinct run
   provenance, `signoff == recommendations.is_empty()`, seal validation, and
   history-only rebase behavior. Do not add shared schema migration,
   cryptographic, or privileged receipt machinery.
5. Teach autopilot, integrator, and delivery prompts to drive discovery once,
   hand the full ledger to implementation, collect batch responses and
   self-verification, and run scoped verification until the selected lifecycle
   roster is unanimously merge-ready.
6. Replace fixed-ten and repeated-round prose in binding docs and the
   constitution with the accepted selected-roster lifecycle. Keep ADR 0055
   unchanged as the historical decision and update the existing changelog
   fragment without process markers.
7. Add the build seat's source guidance and ownership boundary to
   `docs/adr/specs/0053-panel-prompt-sources.md`. Update
   `prompt-corpus.mjs` and the manifest from the old 32-file and
   thirteen-agent assumptions to the exact 35-file and sixteen-agent current
   pool, as pinned by Slice 1's binding tests. Recapture only after all governed
   prompts and contributor documents are final.

### Validation

Run only this enforcing set:

```text
make test-lint
cargo test --manifest-path packages/Cargo.toml -p xtask
cargo clippy --manifest-path packages/Cargo.toml -p xtask --all-targets -- -D warnings
cargo fmt --manifest-path packages/Cargo.toml -p xtask -- --check
make check-tier0
make test-changelog
make test-policy
```

`make test-lint` is the entrypoint for all panel JavaScript tests and binding
checks. `make test-policy` is included only because the expected implementation
changes governed contributor documents; if direct inspection removes every
governed-doc change, omit it and record that fact in validation evidence. No
full Rust workspace, Nix evaluation, container, VM, live-host, or hardware lane
is justified.

### Changed-path allowlist

The final scope guard compares `git diff --name-only "$BASE"...HEAD` with this
closed allowlist. Braces below enumerate literal alternatives; there is no
recursive wildcard and no category-wide `docs/**`, `packages/**`, or runtime
admission.

```text
.specify/feature.json
.specify/memory/constitution.md
AGENTS.md
specs/004-adr0055-panel-review/checklists/requirements.md
specs/004-adr0055-panel-review/{spec,plan,research,data-model,quickstart,tasks}.md
specs/004-adr0055-panel-review/contracts/panel-artifacts.md
.github/agents/d2b-integrator.agent.md
.github/agents/panel-{software,test,product,docs,security,observability,nixos,networking,kernel,rust,simplicity,reliability,agentic,build}.agent.md
.github/skills/d2b-panel-round/SKILL.md
.github/skills/d2b-panel-round/selection-table.json
.github/skills/d2b-panel-round/scripts/{stage-diffs.sh,make-records.mjs,panel-lifecycle.mjs}
.github/skills/d2b-autopilot/SKILL.md
.github/skills/d2b-wave-delivery/SKILL.md
scripts/copilot/{check-bindings.mjs,test-check-bindings.mjs,test-stage-diffs.mjs,test-make-records.mjs,test-panel-lifecycle.mjs,prompt-corpus.mjs,prompt-corpus-manifest.json}
packages/xtask/src/delivery/{model,panel,seal,evidence,command,mod}.rs
tests/test-lint.sh
docs/contributing/{README,panel-review,copilot-agents}.md
docs/adr/specs/0053-panel-prompt-sources.md
docs/specs/ADR-046-validation-and-delivery.md
changelog.d/adr055-panel-review.md
```

The guard expands these alternatives into a literal set, prints every changed
path absent from that set, and exits nonzero when the absent set is non-empty.
It does not merely prove that selected runtime paths stayed clean.

### Mechanically Checkable Done Condition

`spec004w1` is done only when:

1. Every command in the validation list required by the final changed paths
   exits zero.
2. Node.js tests cover every selection trigger and floor, ambiguity widening,
   build citation negatives, deterministic `R` identifiers, complete source
   mapping, all dispositions, conflict refusal, scope refusal, monotonic
   widening, late-issue admission/refusal, metrics including zero denominator,
   prompt scope, and complete/partial repeated legacy import. Planted cases
   refuse a missing selected-seat discovery result, a missing ledger response,
   and incomplete required justification or evidence, while accepting an
   explicit complete zero-finding seat result.
3. Both JavaScript record-generation tests and xtask panel-request tests refuse
   candidate, selection-schema, selection-table-version, and ordered-roster
   mismatches against the same selection artifact.
4. Approval tests accept Fixed BLOCKER, factually verified Invalid and
   Withdrawn BLOCKER, Fixed MAJOR, and maintainer- or merge-owner-accepted
   non-Fixed MAJOR responses. They refuse Intentionally rejected and Deferred
   BLOCKER responses and every unaccepted non-Fixed MAJOR, including Deferred.
5. xtask tests prove two different selected-roster sizes attest only their
   request's exact roles and record files, missing or extra records fail, and
   a schema-version-2 ten-seat request and record set including `rust` remains
   readable. Serialization tests prove no delivery field was added to request,
   record, attestation, or seal.
6. `node scripts/copilot/prompt-corpus.mjs` exits zero after the final capture,
   its shape test requires 35 files and sixteen agent files including `build`
   and excluding current `rust`,
   and a second lifecycle artifact generation produces identical bytes.
7. The work review records `signoff: true` with empty recommendations for every
   selected lifecycle seat; only unresolved `CRITICAL` or `HIGH` merge blockers
   may prevent that state.
8. With `BASE="$(git merge-base origin/v3 HEAD)"`, the changed-path allowlist
   guard above exits zero. Any changed path not in the expanded literal set
   fails the wave.
9. The branch opens one Track B PR for `spec004w1`; there is no delivery seal
   and no partial-cutover PR.

## Concrete Failures and Guards

| Concrete failure | Guard |
| --- | --- |
| Deduplication drops a source finding. | Complete source-to-ledger mapping validation refuses generation. |
| Identical inputs change `R` identifiers or rendered bytes. | Deterministic fixture tests and conflicting-regeneration refusal stop dispatch. |
| One consumer uses a different candidate, selection schema, selection-table version, or roster. | Both panel-request and make-records validate the same selection artifact and planted mismatch cases refuse output. |
| A selected discovery seat is absent but is treated as finding-free. | Complete-result coverage refuses the missing seat; a separate zero-finding positive proves the valid empty shape. |
| A ledger item disappears from implementation responses or carries incomplete justification or evidence. | Exact response coverage and disposition-specific completeness validation refuse verification preparation. |
| Deferred is used to approve a BLOCKER or an unaccepted MAJOR. | Severity-by-disposition approval matrix tests refuse both and positively cover verified factual and recorded-acceptance paths. |
| A fix silently removes a reviewer. | Recorded set-union validation requires monotonic roster widening. |
| A late style issue restarts discovery. | Verification admission rejects non-blocking late issues. |
| Partial legacy import loses completed work or relabels Rust attribution. | Lifecycle import fixtures assert raw-byte preservation, source identity stability, and Rust-profile responsibility. |
| xtask accepts a global default instead of the selected roster. | Request-bound variable-roster tests reject every missing or extra record. |
| Delivery compatibility grows a second schema or silently drops legacy `rust`. | Serialized-field assertions and the ten-seat compatibility case require the existing schema-version-2 shapes and retained Rust role. |
| A changed runtime or other undeclared file escapes a narrow denylist. | The final literal changed-path allowlist fails on every undeclared path. |
| Review expands the accepted scope. | Only `CRITICAL` and `HIGH` merge blockers enter implementation recommendations; all other feedback stays informational. |

## Complexity Tracking

No constitutional violation or additional architectural component is required.
