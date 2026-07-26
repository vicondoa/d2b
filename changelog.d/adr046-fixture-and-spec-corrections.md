### Changed

- Added the fixture-dependent Rust contract lane to the manifest-driven local
  and pull-request test graph as an advisory job, with its skip conditions and
  promotion requirements documented for contributors.
- Changelog fragment parsers now require canonical ASCII dash bullets so
  malformed release-note entries fail consistently.

### Fixed

- Corrected contributor and reference documentation that named retired shell
  gates as current enforcement, distinguishing enforcing coverage from
  fixture-dependent advisory policies and historical evidence.
- Reconciled Integration and Detailed-design contract rows across 30 of the 55
  resource specifications with the decision register, correcting roughly 35
  contradictions in lifecycle, ownership, authorization, placement, and
  provider behavior.
