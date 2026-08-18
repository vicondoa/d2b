### Fixed

- Require the public Network reconciliation path to observe each child
  realization and fail closed instead of asserting child readiness through
  hardcoded inputs.
- Keep the compatibility Network path Pending when durable child-resource
  readiness is unavailable, and preserve the existing typed status resource
  projection while updating only universal phase fields.
- Bound broker-audit evidence replay pages so restart recovery stays within
  the broker transport frame limit.
