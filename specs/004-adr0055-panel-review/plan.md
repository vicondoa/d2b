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
optional `build`, with Rust as a `software` profile. Reselect over the full
candidate and each fix delta and only widen the lifecycle roster. Generate the
discovery, ledger, response, and per-seat verification artifacts
automatically; fail on conflicting regeneration. Automatically import complete
or partial legacy rounds, preserve raw findings and attribution, and keep
metrics informational. Change the existing xtask delivery validator from a
fixed ten-record check to exact validation of the candidate-bound selected
roster while retaining legacy readability.

This is contributor tooling only. It adds no service, daemon, broker surface,
socket, operating-system principal, authorization protocol, cryptographic
receipt, privileged audit path, crate, Gas City implementation, or exhaustive
crash/process machinery.

## Spec Corrections

| Source drift | Canonical state observed before implementation | Treatment |
| --- | --- | --- |
| The feature describes the accepted lifecycle as the target behavior. | `stage-diffs.sh`, `make-records.mjs`, the panel agents, and xtask still implement repeated fixed ten-seat rounds. | Preserve that behavior until every replacement part lands in `spec004w1`; do not ship a half-cutover. |
| Current contributor docs and Constitution Principle VI name a fixed ten-seat roster. | Passing `PANEL_ROLES` validation and those docs agree today. | Update code and every binding contributor document together to the ADR 0055 selected-roster contract. |
| Current delivery records require exactly the legacy ten roles. | `packages/xtask/src/delivery/panel.rs` rejects any other roster. | Dispatch on artifact version before parsing, keep complete legacy records readable, and validate new records against the selected roster stored in their request. |

## Technical Context

**Language/Version**: Existing Node.js ES modules, Bash, repository-pinned Rust,
JSON, and Markdown.

**Primary Dependencies**: Node.js standard library, Git, existing Copilot
skills and read-only agents, and existing xtask dependencies (`serde`,
`serde_json`, and `sha2`). No new package or crate dependency.

**Storage**: Versioned JSON and Markdown in `.scratch/panel/<lifecycle>/` plus
the existing external candidate-addressed delivery state. No database or
privileged storage.

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

**Constraints**: Fail closed on missing seats, mappings, dispositions,
conflicting bytes, roster narrowing, malformed legacy data, unrelated fix
scope, or incomplete merge blockers. Cutover is one PR with no wave seal.

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
- **IV - compatibility**: Pass. Current bytes remain readable through an
  explicit legacy transform, while new lifecycle artifacts have a version
  discriminator and deterministic rendering.
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
├── contracts/
│   └── panel-artifacts.md
├── data-model.md
├── plan.md
├── quickstart.md
├── research.md
└── spec.md
```

The research, data model, contract, and quickstart artifacts record the
accepted design and validation contract.

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
├── test-check-bindings.mjs
├── test-stage-diffs.mjs
├── test-make-records.mjs
├── test-panel-lifecycle.mjs
└── prompt-corpus-manifest.json

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
docs/specs/ADR-046-validation-and-delivery.md
changelog.d/adr055-panel-review.md
```

**Structure Decision**: Keep the behavior in the existing panel skill and its
scripts. Add only one focused JavaScript lifecycle helper and the normative
selection table. Retire the separate `panel-rust` agent only after legacy Rust
attribution maps to the Rust profile on `software`. Extend the existing xtask
crate rather than adding a crate. The listed files are the likely ownership
set from direct inspection; implementation may narrow it or add a directly
referenced file in these same existing surfaces after recording the ownership
adjustment, but may not add an architectural surface.

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
   from this table so prompts cannot drift from selection.
2. Generate discovery requests, merged ledgers, response templates, validation
   and self-review sections, and per-seat verification requests. The
   orchestrator assigns deterministic `R` identifiers after deduplication and
   validates complete source-to-ledger mappings. Require every response to use
   `Fixed`, `Intentionally rejected`, `Deferred`, `Withdrawn`, or `Invalid`
   with the justification and evidence required by ADR 0055.
3. Reselect over the full candidate and every fix delta, union the result with
   the prior lifecycle roster, and reject narrowing or unrelated scope
   expansion with an actionable new-lifecycle remedy.
4. Admit late verification issues only under ADR 0055 rules. Keep MINOR/NIT
   history in the ledger, not in blocking recommendations.
5. Import complete legacy rounds as discovery and partial rounds as preserved
   sources followed by the lifecycle's one current discovery. Dispatch by
   version before parsing, identify a source by record digest, seat, and
   recommendation ordinal, preserve raw bytes and attribution, and map legacy
   `rust` responsibility to the `software` Rust profile. Map exact bracketed
   legacy prefixes `[critical]`, `[high]`, `[medium]`, and `[low]` to
   `BLOCKER`, `MAJOR`, `MINOR`, and `NIT` with ASCII case folding; classify
   every other spelling as migration-assigned `MAJOR`. Preserve imported
   obligations in the roster union and make repeated import byte-stable.
6. Calculate initial, late, severity, review-iteration,
   implementation-iteration, and fixed-per-iteration metrics. Never consult a
   metric when deciding approval.

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
- `docs/specs/ADR-046-validation-and-delivery.md`
- `changelog.d/adr055-panel-review.md`
- `scripts/copilot/prompt-corpus-manifest.json`

**Work**

1. Keep `PanelRole::Rust` readable only for legacy data, add the current seat
   pool, bind each new request to its selected ordered roster, and require
   exactly one unanimous record for every role in that request. Seal and
   evidence checks use the attested selected roster rather than a global count.
2. Preserve model/effort checks, candidate binding, distinct run provenance,
   `signoff == recommendations.is_empty()`, and history-only rebase behavior.
   Do not add cryptographic or privileged receipt machinery.
3. Teach autopilot, integrator, and delivery prompts to drive discovery once,
   hand the full ledger to implementation, collect batch responses and
   self-verification, and run scoped verification until the selected lifecycle
   roster is unanimously merge-ready.
4. Replace fixed-ten and repeated-round prose in binding docs and the
   constitution with the accepted selected-roster lifecycle. Keep ADR 0055
   unchanged as the historical decision and update the existing changelog
   fragment without process markers.
5. Recapture the checked-in prompt corpus only after all governed prompt and
   contributor documents are final.

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

### Mechanically Checkable Done Condition

`spec004w1` is done only when:

1. Every command in the validation list required by the final changed paths
   exits zero.
2. Node.js tests cover every selection trigger and floor, ambiguity widening,
   build citation negatives, deterministic `R` identifiers, complete source
   mapping, all dispositions, conflict refusal, scope refusal, monotonic
   widening, late-issue admission/refusal, metrics including zero denominator,
   prompt scope, and complete/partial repeated legacy import.
3. xtask tests prove two different selected-roster sizes attest only their
   request's exact roles, missing or extra records fail, legacy fixed-ten
   records remain readable, and the seal carries the selected roster.
4. `node scripts/copilot/prompt-corpus.mjs` exits zero after the final capture,
   and a second lifecycle artifact generation produces identical bytes.
5. The work review records `signoff: true` with empty recommendations for every
   selected lifecycle seat; only unresolved `CRITICAL` or `HIGH` merge blockers
   may prevent that state.
6. With `BASE="$(git merge-base origin/v3 HEAD)"`,
   `git diff --quiet "$BASE"...HEAD -- nixos-modules packages/d2bd
   packages/d2b-priv-broker packages/d2b-contracts packages/Cargo.toml` exits
   zero, proving the wave added no runtime, broker, contract, or crate surface.
7. The branch opens one Track B PR for `spec004w1`; there is no delivery seal
   and no partial-cutover PR.

## Concrete Failures and Guards

| Concrete failure | Guard |
| --- | --- |
| Deduplication drops a source finding. | Complete source-to-ledger mapping validation refuses generation. |
| Identical inputs change `R` identifiers or rendered bytes. | Deterministic fixture tests and conflicting-regeneration refusal stop dispatch. |
| A fix silently removes a reviewer. | Recorded set-union validation requires monotonic roster widening. |
| A late style issue restarts discovery. | Verification admission rejects non-blocking late issues. |
| Partial legacy import loses completed work or relabels Rust attribution. | Exact legacy fixtures assert raw-byte preservation, source identity stability, and Rust-profile responsibility. |
| xtask accepts a global default instead of the selected roster. | Request-bound variable-roster tests reject every missing or extra record. |
| A partial cutover makes current and new artifacts mutually unreadable. | One atomic PR plus version-first compatibility tests; existing behavior remains canonical until merge. |
| Review expands the accepted scope. | Only `CRITICAL` and `HIGH` merge blockers enter implementation recommendations; all other feedback stays informational. |

## Complexity Tracking

No constitutional violation or additional architectural component is required.
