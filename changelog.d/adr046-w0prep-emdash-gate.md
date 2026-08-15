### Added

- `make check-tier0` now scans the whole repository for every non-ASCII dash
  codepoint (U+2010, U+2011, U+2012, U+2013, U+2014, U+2015, U+2212, U+FE58,
  U+FF0D) and fails closed with every offending `file:line`. The scan covers
  every tracked file plus every non-ignored untracked file, skips binaries, and
  adds under 100ms to the gate.

### Changed

- Only the plain ASCII hyphen `-` may now spell a dash anywhere in the
  repository. Every non-ASCII dash codepoint is banned and every existing
  occurrence was rewritten: a dash that separated clauses became a spaced
  hyphen ` - `, and a dash that joined a range or a compound closed up to `-`.
  Documentation, specs, ADRs, comments, CLI text, and generated artifacts are
  all affected.
