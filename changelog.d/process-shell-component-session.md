### Changed

- Route guest execution and persistent shell lifecycle through Process-family resource intents and authenticated ComponentSession named streams while retaining bounded authorization, retention, and disconnect handling; unavailable target-session routes fail closed without a legacy fallback.
- Demultiplex concurrent named-stream responses, cancel/reset on owner disconnect, and use one correlated JSON frame codec across CLI, ResourceClient, Providers, and daemon shell owners.
- Reject the retired public `exec`/guest shell bridge in production composition and record target-local supervisor Process ownership for ShellSession resources.
- Resolve enrolled target-local ComponentSessions for real Process and ShellSession routes, and persist shell supervisor Process ownership through the daemon Resource API with exact owner and finalizer fencing.

### Fixed

- Preserve the development-shell PATH for local Bazel test runners without
  overriding the worker-standard action PATH used by remote profiles.
