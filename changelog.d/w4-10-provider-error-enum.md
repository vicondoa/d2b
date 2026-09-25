### Fixed

- The notification-desktop provider now returns the typed `ProviderError`
  enum instead of `&'static str` reason codes from its public constructors,
  validators, reconciliation methods, and the source-process and lifecycle
  effect-port traits, so callers can match failures without string
  comparison. The stable error slugs are unchanged.