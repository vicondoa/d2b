### Changed
- Enforce Provider EffectPort, specification vocabulary, test-taxonomy, deterministic-time, and current threat-matrix policies.
- Extend interaction canary coverage across clipboard, terminal, notifications, and security-key CTAP surfaces.
- Add zero-secret Credential canaries for merged Entra, managed identity, and Secret Service configuration, placement, session, lease, ambiguous-completion, recovery, audit, telemetry, Debug, and support surfaces.
- Extend the security matrix across the U4 cutover/reset contracts while keeping live drain and effect execution explicitly pending on the unfinished U4 runtime boundary.

### Fixed
- Redact CTAPHID report bytes from frontend Debug output while retaining privileged audit fail-closed and quarantine evidence.
