### Changed

- Bound Entra credential acquisition, refresh, authorization, redacted status,
  bounded degradation, and finalization cleanup to the owning Guest and Zone.
- Require exact authenticated Guest, Provider, and Zone bindings before any
  client effect, preserve committed refresh metadata after post-commit
  validation failures, and prevent finalized credentials from minting again.
- Bind every service operation to the authenticated Credential session and
  delivery context, persist remote lease state, reject generation rollback,
  accept Unix-millisecond deadlines, and cap projected retry state.
- Revoke newly issued leases when replacement validation or local commitment
  fails, retaining degraded cleanup state when remote revocation is ambiguous.
