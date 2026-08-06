### Added

- Added pinned, inert Caveman prompt sources and Copilot-native optional
  communication and compression skills. The integration uses the active
  session only and does not require an external runtime, installation,
  network access, or content upload.
- Added `d2b-spec-edit` as the exclusive route for later changes inside an
  active feature directory, with fail-closed root checks, batched edits, and
  changed-path verification.
- Added a checked-in prompt corpus manifest and preservation checker covering
  prompt structure, commands, paths, identifiers, negations, tables, and
  output schemas.

### Changed

- Extended binding and policy checks for the pinned vendor files, optional
  communication declarations, feature-artifact ownership, panel invariants,
  and prompt corpus membership.
- Extended the dash policy with an exact path and hash admission for the
  pinned vendor files while retaining the repository-wide prohibition
  elsewhere.
