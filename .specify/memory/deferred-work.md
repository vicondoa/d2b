# Deferred work

Work a wave consciously chose not to do. Managed by the `d2b-memory` skill.

**Classification metadata only.** No transcripts, no validation output, no
attestation payloads, no diffs. If an entry needs a paragraph of context to be
actionable, it is a task and belongs in a plan.

Wave addresses use the qualified token (`spec001w1`, `adr046w3fu2`). A legacy
bare `W0` through `W8` remains valid and means program `ADR046`.

Categories: `signoff`, `build`, `test`, `merge`, `codegen`, `disk`.
Dispositions: `open`, `folded`, `filed`, `resolved`, `wontfix`.

Critical and high panel findings are never deferrable and never appear here.
They are fixed in the round that raised them.

| Wave | Category | Date | Statement | Disposition | Ref |
|---|---|---|---|---|---|
| copilotw3 | build | 2026-07-31 | `specify init` not run in-repo; skills imported additively instead, so a spec-kit upgrade needs the same manual import | open |  |
| copilotw6fu4 | test | 2026-07-31 | `test-check-bindings.mjs` covers the seat-roster guard only; the scalar constant mirrors share a loop and have no negative case | open |  |
| copilotw6fu6 | test | 2026-07-31 | `REQUIRED_INPUTS` / `OPTIONAL_INPUTS` classification is asserted by comment only; the omit-each-input probe that verified it against the gate was run by hand and not committed | open |  |
