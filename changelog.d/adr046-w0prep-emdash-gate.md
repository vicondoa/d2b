### Added

- `make check-tier0` now scans the whole repository for the em-dash character
  (U+2014) and fails closed with every offending `file:line`. The scan covers
  every tracked file plus every non-ignored untracked file, skips binaries, and
  adds under 100ms to the gate.

### Changed

- Banned the em-dash character (U+2014) repository-wide and rewrote every
  existing occurrence as a spaced hyphen. Documentation, specs, ADRs, comments,
  CLI text, and generated artifacts all read with ` - ` where they previously
  carried an em-dash. The en-dash (U+2013) is deliberately unaffected.
- The ADR-046 spec-registry census now labels its dashed-title bucket
  `dash title` instead of `em-dash title`. The bucket counts titles that split
  an identifier from its prose with a dash of any kind; only the spelling of
  the label changed, not the classification or the count.
