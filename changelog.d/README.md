# `changelog.d/` — changelog fragments

`CHANGELOG.md` has one `## [Unreleased]` block and every branch appends to the
bottom of it, so any two branches in flight collide there even when their
entries are completely independent. The conflicts are trivial to read and still
have to be resolved by hand, one pair at a time.

This directory removes that collision. A branch does not edit `CHANGELOG.md`;
it drops one fragment file here. Two branches never write the same file, so the
changelog stops generating merge conflicts. The integrator folds the fragments
into `CHANGELOG.md` when the work merges.

## Naming rule

One file per branch, named after the branch:

```
changelog.d/<branch-name>.md
```

For a branch named `fix-store-farm-exdev` that is
`changelog.d/fix-store-farm-exdev.md`. The name only has to be unique and
stable for the life of the branch; the fold sorts fragments by file name, so
the name also fixes the entry order within each section.

`README.md` is this document and is never folded.

## Format

Standard [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) section
headings with your entries under them:

```markdown
### Added

- Added the thing, and what an operator can now do with it.

### Fixed

- Fixed the thing that misbehaved, and the symptom that is now gone.
```

Rules, all enforced fail-closed — a fragment that breaks one of them aborts the
fold with the file and line rather than silently dropping the entry:

- Only `### Added`, `### Changed`, `### Deprecated`, `### Removed`,
  `### Fixed`, and `### Security` headings, at exactly that heading level.
- No content before the first heading.
- No repeated heading within one fragment.
- Every section carries at least one entry and starts with a `- ` bullet.
- A fragment carries at least one section; an empty file is rejected.

Entry lines are copied verbatim, so multi-line entries, indented continuation
lines, and nested bullets survive the fold unchanged. Write entries the way a
consumer reads them: what changed and why it matters, not which branch or
process step produced it.

## Folding

The integrator runs the assembler from the repository root:

```bash
cd packages && cargo run -q -p xtask -- changelog-fold      # or: make changelog-fold
```

It merges every fragment into the `## [Unreleased]` block by section — all
`### Added` bullets from all fragments collate under one `### Added` — in
Keep a Changelog order (`Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`,
`Security`), appends to whatever the block already carried, leaves released
version sections untouched, and deletes the fragments it consumed. Fragments
are read in file-name order, so the result is byte-stable for a given set of
fragments. With no fragments present the changelog is not rewritten at all, so
running the fold twice is a no-op.

`cargo run -q -p xtask -- changelog-fold --check` validates every fragment and
computes the same fold without writing or deleting anything.

## The changelog gate

`scripts/changelog-check.sh` (the `test-changelog` job) requires a code change
to ship release notes. A pull request satisfies it with **either** an entry in
`CHANGELOG.md` **or** a fragment in this directory — neither is not an option.
The same gate validates the structure of every fragment present, so a malformed
fragment fails on the pull request that introduced it rather than at fold time.
