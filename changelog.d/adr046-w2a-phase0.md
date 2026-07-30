### Changed

- The ADR-046 binding ten-role panel is bound to a reviewing model that is
  deliberately distinct from the model used to write the code, so a lane
  cannot both author a change and attest to it. Attestation rejects a record
  carrying the coding model.
- Wave entry no longer requires every prior-wave work item to be merged.
  That condition now binds at the wave exit boundary instead - panel request,
  seal, and merge eligibility - which is what makes a pipelined wave start
  executable. Panel, seal, and merge remain strictly ordered between waves.
- The delivery contract records that a wave's work reaches the integration
  lineage only through pull requests passing its gates, and that rework
  caused by starting a wave early is absorbed by the wave that started
  early rather than used to shorten its predecessor's panel.

### Added

- The code-review diff base can be selected explicitly, and an explicit base
  that does not resolve is a hard error rather than a silent fallback to the
  repository default branch.
- Every failure the diff-base selection can produce now names a concrete
  recovery step rather than only reporting what failed, and reports it in the
  caller's requested output format so a machine consumer can parse it.

### Fixed

- Replay no longer decodes every revision-log entry before discarding it, and
  a change batch is materialized once and shared across matching watchers
  rather than deep-copied per watcher. At ten thousand resources and one
  hundred live watches this brings whole-process resident memory within its
  budget, with per-watch cost falling to a fraction of a kibibyte.
