### Added

- Added Secret Service, Entra identity-Guest, and managed-identity Credential Providers with exact-consumer admission, immutable delivery bindings, opaque lease lifecycle handling, placement validation, and process-unique redaction canaries.

### Security

- Kept credential bytes inside injected secret-holding clients and dedicated authenticated delivery sessions, with no Provider-selected consumer, audience, route, or delivery limit and no secret or Credential identity in diagnostics or telemetry.
