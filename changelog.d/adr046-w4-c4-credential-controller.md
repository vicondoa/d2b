### Added

- Added a neutral Credential controller contract for exact operation authorization, bounded active-lease capacity, rotation and revocation decisions, scheduled metadata observation, and single-flight reconciliation. It has no production controller caller, and its complete policy-and-outcome decision matrix is still pending.
- Added bounded Credential audit record and telemetry frame builders with structural redaction tests. Production service and controller paths do not yet call these builders or emit them to Zone audit and telemetry sinks.
