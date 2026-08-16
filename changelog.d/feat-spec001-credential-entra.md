### Changed

- Bound Entra credential acquisition, refresh, authorization, redacted status,
  bounded degradation, and finalization cleanup to the owning Guest and Zone.
- Require exact authenticated Guest, Provider, and Zone bindings before any
  client effect, preserve committed refresh metadata after post-commit
  validation failures, and prevent finalized credentials from minting again.
