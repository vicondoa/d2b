### Added

- Added a fail-closed Provider crate layout policy that uses Cargo metadata,
  checks the normative source, test, integration, and README contract, and
  rejects Provider crates omitted from the workspace.
