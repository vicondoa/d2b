### Added

- Added `changelog.d/`, a changelog-fragment directory, so concurrent branches
  no longer collide in the single `## [Unreleased]` block of `CHANGELOG.md`.
  Each branch writes one `changelog.d/<branch>.md` file holding standard
  Keep a Changelog `### <Section>` headings and entries; `changelog.d/README.md`
  documents the naming rule, the accepted format, and the fold.
- Added `cargo xtask changelog-fold`, which merges every fragment into the
  `## [Unreleased]` block by section in Keep a Changelog order, appending to
  the sections already present, leaving released versions untouched, and
  deleting the fragments it consumed. Fragments are folded in file-name order
  for a byte-stable result, a run with no fragments leaves the changelog
  untouched, and a fragment with an unknown heading, a repeated heading, an
  empty section, or content outside a section aborts the run with the offending
  file and line instead of dropping the entry. `--check` validates and computes
  the fold without writing.

### Changed

- The changelog policy gate now accepts release notes as either a `CHANGELOG.md`
  entry or a `changelog.d/` fragment, and additionally validates the structure
  of every fragment present so a malformed fragment fails on the pull request
  that introduced it rather than when the fragments are folded.
