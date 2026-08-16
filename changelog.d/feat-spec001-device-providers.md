### Fixed

- Recovered device TPM workers safely after confirmed stale-process crashes without risking duplicate workers during ambiguous liveness checks.
- Enforced persisted device ownership, zone, and physical-key exclusivity before opening security-key relays.
- Preserved typed TPM directory-hardening audit records and established the swtpm control endpoint before initialization.
