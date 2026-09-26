### Added

- `NotificationCutoverState`, the typed cutover state of the
  notification-desktop `NotificationRunnerContract`.

### Changed

- `d2b-provider-process-systemd` exports only the surface production
  composes today: the Provider controller, the `lifecycle` root
  re-exports, `effects_service`, and `operations`. The controller
  family, `drain`, `metrics`, `audit`, `launch`, and `sandbox` modules
  have no production consumer - the conformance tests exercise the first
  three - so they compile behind the crate's `test-support` feature and
  are gone from the plain library build. The tests consuming them
  declare `required-features` and run with `--features test-support`.
- The notification-desktop `NotificationRunnerContract` replaces its two
  always-true cutover flags with one `NotificationCutoverState`.
  `component_session_only` and `watched_configuration_is_dependency`
  remain as reads of that state, so every existing caller is unchanged.
