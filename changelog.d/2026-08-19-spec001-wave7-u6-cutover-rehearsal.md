### Added

- Add the U6 booted-VM cutover rehearsal, candidate-bound live driver,
  safe cutover runbook, and security manual-validation checklist. The live
  lane validates recovery and delivery evidence through production validators,
  stops at `CutoverSucceeded`, and never performs phase-10 finalization.
