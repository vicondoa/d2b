### Fixed

- Require the public Network reconciliation path to commit and re-read a
  typed child-readiness projection before publishing durable Ready or
  launching a dependent Guest.
- Preserve typed status resource projections while updating universal phase
  fields, and classify pre-upgrade restore backups as upgrade-required.
- Bound broker-audit evidence replay pages so restart recovery stays within
  the broker transport frame limit.
