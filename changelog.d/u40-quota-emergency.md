### Added

- Added zone-wide quota enforcement to the resource graph: a hard `Quota`
  ceiling now refuses an over-limit resource before anything is persisted, so
  an over-quota mutation leaves no desired row behind and grants no effect. A
  soft ceiling reports the excess on the row's own status, and a mutation that
  only removes use is never refused, so a Zone can always recover from its own
  excess.
- Added emergency-policy enforcement to the resource graph: an accepted
  `EmergencyPolicy` reduction now blocks new use at the revoking stage and
  drives the use that already exists to its safe state in order - new use is
  fenced, each consumer is detached while its helper legs still exist, and the
  source reservation is released last.
- Added `d2bd::GraphLimitsAdmission`, the composition that decides who is
  asking through the one shared graph evaluator and then defers the limit and
  emergency decisions to the two owning provider crates, for the mutation,
  effect, and drain boundaries.
- Added `RefusalReason::EmergencyReductionActive` so an emergency block is
  distinguishable from a quota ceiling in a diagnostic.

### Changed

- Changed a broker that cannot answer its open-use census to leave the Zone
  fenced with conservative ownership: the reduction still blocks new use,
  nothing is reported as drained, and no reservation may be released, instead
  of the reduction appearing to have converged on a guess.
- Changed a `Quota` or `EmergencyPolicy` row to be exempt from its own policy,
  so a Zone can always lower or clear the limits and the emergency that are
  blocking it.
