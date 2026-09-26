### Fixed

- The unsafe-local helper no longer reports a scope whose launched process
  exited during startup (or that is stopping or degraded) as an identity
  mismatch. Such early-exit scopes are now reported as
  `unsafe-local-shell-scope-create-failed` (wire code 42) instead of
  `unsafe-local-shell-scope-identity-mismatch` (wire code 70), so an
  operational startup failure is not surfaced as a security identity failure.