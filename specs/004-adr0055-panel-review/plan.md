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
Keep workspace delivery schema version 2, but add the smallest panel-specific
discriminator: current request, record, attestation, and the seal's embedded
panel object carry `panel_format_version: 1`; legacy fixed-ten artifacts omit
it. Probe bounded JSON before selecting strict legacy or current DTOs. The
current format supports the expanded role domain, while legacy remains exactly
the historical ten roles including `rust`.

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
| Every delivery artifact currently uses workspace schema version `2` and no panel-specific discriminator. | Existing fixed-ten request, record, attestation, and seal panel objects cannot be distinguished safely from the selected-roster format if only their shared schema version is inspected. | Keep `DELIVERY_SCHEMA_VERSION = 2`; add `panel_format_version: 1` only to current panel request, record, attestation, and the seal's embedded panel object. Legacy fixed-ten DTOs omit it and retain `rust`. |
| The checked prompt corpus enforces an exact 32-file shape with thirteen agent files, and ADR 0053's prompt-source specification describes the superseded twelve-role, `relevant`/`signoff`/`recommendations`/`prior_resolutions` verdict, held-reviewer, repeated-round lifecycle with no build seat. | `prompt-corpus.mjs`, its binding tests, and `docs/adr/specs/0053-panel-prompt-sources.md` agree with the current pre-ADR-0055 contract. | Replace or explicitly withdraw every superseded fixed-roster, four-field verdict, held-reviewer, repeated-round, and old verification contract in that source document; make the selected roster, complete discovery results, shared ledger and responses, and scoped verification operative; add build guidance; and pin the corpus and tests to 35 files with sixteen agent files in the same atomic cutover. |

## Adjudication

The round-2 security recommendation to make maintainer or merge-owner
acceptance authoritative is rejected. Accepted ADR 0055 explicitly classifies
panel artifacts as bypassable process records rather than authorization.
Acceptance remains a plain recorded response under ordinary repository
controls. The implementation shape-checks it but adds no identity verification,
signature, GitHub API lookup, service, protected principal, or authority.

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
selection-table-version, panel-format-version, family, or ordered-roster
mismatch; missing seat results, mappings, or ledger responses; incomplete
required justification, evidence, factual verification, or acceptance when
required; conflicting bytes; roster narrowing; malformed legacy data;
unrelated fix scope; or incomplete merge blockers. Every delivery JSON read is
bounded before discriminator probing and strict DTO parsing. Cutover is one PR
with no wave seal.

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
- **IV - compatibility**: Pass. Workspace delivery schema version 2 remains
  unchanged. Current panel request, record, attestation, and seal panel objects
  add only `panel_format_version: 1`; strict legacy DTOs omit it and preserve
  exactly the fixed ten including `rust`. A bounded version-first probe
  prevents ambiguous fallback.
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
├── mod.rs
└── testdata/
    ├── panel-legacy-ten.json
    └── panel-current-variable.json

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
crate rather than adding a crate. Add only strict legacy/current panel DTOs and
two compact fixture bundles; do not create a parallel workspace schema, generic
migration framework, or fixture family. The ownership set below is closed for
this wave; an undeclared changed path fails the final allowlist instead of being
admitted by an implementation-time adjustment.

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
   A MAJOR approves as Fixed or as factually verified Invalid or Withdrawn
   without acceptance. Only unresolved Intentionally rejected or Deferred
   MAJOR responses require recorded acceptance by the repository maintainer or
   merge owner. A valid acceptance object requires a non-empty accepter
   identifier, capacity exactly `repository maintainer` or `merge owner`, and a
   non-empty acceptance justification. That acceptance is plain shape-checked
   process data, not verified identity or authorization. Deferred cannot
   approve a BLOCKER or an unaccepted MAJOR.
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
   records. Emit current schema-version-2 `PanelRecord` objects with
   `panel_format_version: 1` for exactly those roles. Preserve model and effort
   checks, candidate binding, distinct run provenance, and
   `signoff == recommendations.is_empty()`.

### Slice 2 - Delivery and Documentation Integration

**Owned files**

- `packages/xtask/src/delivery/{model,panel,seal,evidence,command,mod}.rs`
- `packages/xtask/src/delivery/testdata/panel-legacy-ten.json` (new)
- `packages/xtask/src/delivery/testdata/panel-current-variable.json` (new)
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
   populate the schema-version-2 request's `roles` and `record_files`, with
   `panel_format_version: 1`. Keep lifecycle selection version 1 and the same
   selection path flowing to both `panel-request` and `make-records.mjs`.
2. Read each request, record, attestation, and seal through the existing bounded
   JSON path. Probe the top-level `panel_format_version`, or the seal's nested
   `panel.panel_format_version`, before DTO deserialization. Absence selects a
   strict legacy DTO; integer `1` selects a strict current DTO; malformed,
   unknown, or mixed families fail without fallback. Both families deny unknown
   fields.
3. Give current request, record, attestation, and the seal's embedded panel
   object `panel_format_version: 1`. Keep the top-level seal field set and
   `DELIVERY_SCHEMA_VERSION = 2`. The current role domain is the exact
   thirteen-seat selection pool and excludes `rust`; the legacy domain remains
   the exact historical fixed ten including `rust`.
4. Make `panel-attest` validate exactly the ordered roles and record files
   already stored in a current request, while a legacy request remains exact
   fixed-ten and may finish only with legacy records and a legacy attestation.
   Pin exactly two compact fixture bundles: one legacy ten-seat set and one
   current variable-roster set, each covering request, records, attestation,
   and the seal panel object.
5. Preserve model and effort checks, candidate binding, distinct run
   provenance, `signoff == recommendations.is_empty()`, seal validation, and
   history-only rebase behavior. Do not add a new workspace schema, generic
   migration or fixture framework, identity verification, signatures, GitHub
   API lookup, cryptographic or privileged receipt machinery, service, or
   authority.
6. Teach autopilot, integrator, and delivery prompts to drive discovery once,
   hand the full ledger to implementation, collect batch responses and
   self-verification, and run scoped verification until the selected lifecycle
   roster is unanimously merge-ready.
7. Replace fixed-ten and repeated-round prose in binding docs and the
   constitution with the accepted selected-roster lifecycle. Keep ADR 0055
   unchanged as the historical decision and update the existing changelog
   fragment without process markers.
8. In `docs/adr/specs/0053-panel-prompt-sources.md`, replace or explicitly
   withdraw every superseded fixed-roster,
   `relevant`/`signoff`/`recommendations`/`prior_resolutions` verdict,
   held-reviewer, repeated-round, and old verification contract across its
   metadata, shared contract, stage/seam guidance, local-conflict table,
   caveats, and change discipline. Make the selected roster, complete discovery
   results, shared ledger and responses, and scoped verification operative. Add
   the build seat's source guidance and ownership boundary.
   Update `prompt-corpus.mjs` and the manifest from the old 32-file and
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
packages/xtask/src/delivery/testdata/{panel-legacy-ten,panel-current-variable}.json
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
   candidate, selection-schema, selection-table-version, panel-format-version,
   and ordered-roster mismatches against the same selection artifact.
4. Approval tests accept Fixed BLOCKER, factually verified Invalid and
   Withdrawn BLOCKER; Fixed, factually verified Invalid, and factually verified
   Withdrawn MAJOR without acceptance; and maintainer- or merge-owner-accepted
   Intentionally rejected and Deferred MAJOR responses. They refuse
   Intentionally rejected and Deferred BLOCKER responses, unverified Invalid
   or Withdrawn responses, and unaccepted Intentionally rejected or Deferred
   MAJOR responses. For each of the two unresolved MAJOR dispositions, separate
   planted malformed-acceptance cases refuse an empty accepter identifier, a
   capacity other than `repository maintainer` or `merge owner`, and an empty
   acceptance justification.
5. xtask tests prove two different current selected-roster sizes attest only
   their request's exact roles and record files; missing, extra, out-of-order,
   malformed-version, unknown-version, and mixed-family records fail; and the
   two compact fixtures parse as strict legacy fixed-ten including `rust` and
   strict current variable-roster data. Serialized-field assertions prove
   current request, record, attestation, and the seal panel object carry
   `panel_format_version: 1`, legacy counterparts omit it, the top-level seal
   shape is unchanged, and workspace schema version remains `2`.
6. `node scripts/copilot/prompt-corpus.mjs` exits zero after the final capture,
   its shape test requires 35 files and sixteen agent files including `build`
   and excluding current `rust`; focused source-contract checks prove
   `docs/adr/specs/0053-panel-prompt-sources.md` replaces or explicitly
   withdraws the fixed-roster, four-field verdict, held-reviewer,
   repeated-round, and old verification contracts; and a second lifecycle
   artifact generation produces identical bytes.
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
| A factually verified Invalid or Withdrawn MAJOR is wrongly held for acceptance, or an unresolved Intentionally rejected or Deferred MAJOR passes without it. | Severity-by-disposition tests positively cover resolved factual responses without acceptance and require recorded acceptance only for the two unresolved dispositions. |
| A malformed acceptance object approves an unresolved Intentionally rejected or Deferred MAJOR. | Planted negatives for each disposition refuse an empty accepter identifier, an out-of-domain capacity, and an empty acceptance justification. |
| A fix silently removes a reviewer. | Recorded set-union validation requires monotonic roster widening. |
| A late style issue restarts discovery. | Verification admission rejects non-blocking late issues. |
| Partial legacy import loses completed work or relabels Rust attribution. | Lifecycle import fixtures assert raw-byte preservation, source identity stability, and Rust-profile responsibility. |
| xtask accepts a global default instead of the selected roster. | Request-bound variable-roster tests reject every missing or extra record. |
| A malformed current artifact falls back to legacy parsing, or a current record is mixed into a legacy request. | Bounded discriminator probes select exactly one strict DTO family; malformed, unknown, and mixed-family fixture negatives fail without fallback. |
| Delivery compatibility grows a new workspace schema or silently drops legacy `rust`. | Serialized-field assertions keep workspace schema version 2, and the compact legacy fixture requires exact fixed-ten ordering including `rust`. |
| The prompt-source document adds `build` but leaves old roster, verdict, held-seat, repeated-round, or verification requirements operative. | Focused prompt-source contract checks require the current lifecycle markers and reject the superseded operative clauses unless explicitly withdrawn. |
| Maintainer acceptance becomes an identity or authorization boundary. | Contract tests validate only the plain response shape; the owned-file set has no signature, GitHub API, service, principal, or authority surface. |
| A changed runtime or other undeclared file escapes a narrow denylist. | The final literal changed-path allowlist fails on every undeclared path. |
| Review expands the accepted scope. | Only `CRITICAL` and `HIGH` merge blockers enter implementation recommendations; all other feedback stays informational. |

## Complexity Tracking

No constitutional violation or additional architectural component is required.
