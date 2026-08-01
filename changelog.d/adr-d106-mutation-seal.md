### Added

- Recorded the architecture decision that makes resource-store writes
  reachable only through single-use authorization evidence owned by the
  storage contract crate, minted by the authorization evaluator and bound to
  one store instance, so no other crate can construct or replay it.
