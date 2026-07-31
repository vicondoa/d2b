### Changed

- Restructured `AGENTS.md` from a 122KB monolith into a ~35KB index that
  carries the binding rules and links to detail under `docs/contributing/`.
  No rule was removed; rationale and reference depth moved. Fixed the empty
  `## Development workflow` heading whose subsections were mis-nested under
  `## Changelog & Releases`, and merged the duplicated changelog section.

### Added

- `docs/contributing/` with workflow, panel review, changelog and commit
  conventions, gates and lints, critical subsystems, and architecture
  conventions.
- A context-budget assertion and a link-resolution check for `AGENTS.md`, so
  detail lands in `docs/contributing/` instead of re-growing the file that is
  loaded into every agent session.
- Retired-surface policy scanning now also covers `docs/contributing/`, which
  keeps the ADR 0015 coverage that would otherwise have been lost when the
  prose moved out of `AGENTS.md`.

### Fixed

- `scan_process_markers` now lists `docs/contributing/*` explicitly in its
  exempt arm. Its `case` statement has no default arm, so the path would
  otherwise have been unclassified and exempt by accident rather than by
  decision.
