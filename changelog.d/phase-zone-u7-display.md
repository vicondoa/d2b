### Changed

- Bind the Wayland display proxy to neutral provider contracts while retaining
  authenticated target routing, readiness fencing, and display capabilities.

### Removed

- Remove VM and realm-target compatibility inputs from the display proxy,
  including legacy clipboard bridge metadata and retired owner dependencies.
