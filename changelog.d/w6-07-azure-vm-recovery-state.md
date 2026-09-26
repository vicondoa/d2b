### Changed

- The Azure VM guest controller's sealed restart-recovery record now groups
  the in-flight ARM operation and its start time into one
  `inFlightOperation` object instead of the `operation` +
  `operationStartedAtUnixMs` pair. Records written before the grouping still
  load: the read path accepts the legacy pair shape and folds it into the
  grouped shape, so sealed recovery records written by older controllers are
  unaffected.