### Fixed

- Network spec parse failures in the trusted-bundle network path now surface
  as `manifest-parse-error` instead of an opaque intent-not-found refusal.
  The six `resolve_network_*_intent` resolver methods return a typed error
  when a Network row's spec cannot be parsed (the daemon and the
  Network-local provider keep their closed refusal codes but journal the
  manifest-parse-error reason), and the bulk fixture-intent builder fails
  closed on the same drift instead of silently dropping every intent for
  the affected network, so a producer-side spec drift is diagnosable at
  apply time.