### Fixed

- The Credential controller now emits the documented wire code
  `credential-already-running` when the same Credential is already being
  handled, instead of reusing the lease-ceiling code `credential-queue-pressure`.
  The lease-ceiling outcome now emits its documented code
  `credential-queue-pressure` in audit records and telemetry, matching the
  closed outcome set in ADR-046-resources-credential.