### Changed

- Restructured `AGENTS.md` from a 122KB monolith into a ~38KB index that
  carries the binding rules and links to detail under `docs/contributing/`.
  No rule was removed; rationale and reference depth moved. Fixed the empty
  `## Development workflow` heading whose subsections were mis-nested under
  `## Changelog & Releases`, and merged the duplicated changelog section.

### Added

- Thirteen Copilot agents under `.github/agents/`: `d2b-architect`,
  `d2b-implementer`, `d2b-integrator`, and one per panel seat. Each pins its
  own model in frontmatter, and the ten panel seats declare a read-only tool
  set so a reviewer cannot run a build.
- `d2b-panel-round`, `d2b-wave-delivery`, `d2b-memory`, `d2b-adr` and
  `d2b-autopilot` skills under `.github/skills/`, each carrying a committed
  dispatch binding table.
- `scripts/copilot/check-bindings.mjs`, which rejects an agent with no binding
  row, an effort a model does not support, a panel row disagreeing with the
  delivery policy constants, a panel agent granted write tools, and any effort
  or context-tier key in agent frontmatter. It also reads the delivery-memory
  registers, and refuses one whose rows it could not parse, so a truncated or
  malformed register fails rather than passing as empty. A register that is
  empty on purpose declares it with a marker line documented in the `d2b-memory`
  skill.
- Delivery memory registers under `.specify/memory/` for deferred work,
  engineering friction, and accepted debt.
- spec-kit is installed for Copilot in skills mode alongside the existing
  opencode integration, as `/speckit-<name>` commands under
  `.github/skills/`. Both integrations are listed in `.specify/integration.json`,
  and `check-bindings.mjs` fails if either is dropped.
- `scripts/copilot/autopilot.sh` for unattended runs, pinning the session
  binding, continuation and credit ceilings, and log destination. It refuses a
  dirty worktree, refuses a protected branch, and validates the binding tables
  before the session starts.
- ADR 0048 recording the measured Copilot substrate, the panel and autopilot
  design, and why the model binding is indirected through dispatch parameters.
- `docs/contributing/copilot-agents.md` describing the agent and skill surface,
  the authoring and execution sequences, and the spec-kit coexistence rules.
- `docs/contributing/` with workflow, panel review, changelog and commit
  conventions, gates and lints, critical subsystems, and architecture
  conventions.
- A context-budget assertion and a link-resolution check for `AGENTS.md`, so
  detail lands in `docs/contributing/` instead of re-growing the file that is
  loaded into every agent session.
- Retired-surface policy scanning now also covers `docs/contributing/`, which
  keeps the ADR 0015 coverage that would otherwise have been lost when the
  prose moved out of `AGENTS.md`.
- Delivery waves may now be addressed by a qualified lowercase token fusing
  program and wave, such as `spec001w1`. The delivery state layout does not
  carry the program as a path component, so two programs would otherwise share
  a wave directory. The legacy `--program ADR046 --wave W1` form and every
  existing `W0`..`W8` state directory are unchanged and are not deprecated.

### Fixed

- `scan_process_markers` now lists `docs/contributing/*` explicitly in its
  exempt arm. Its `case` statement has no default arm, so the path would
  otherwise have been unclassified and exempt by accident rather than by
  decision.
