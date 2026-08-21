### Fixed

- Isolate unsafe-local shell supervisor tests from host login profiles so
  Layer-1 Rust checks do not flake on user startup hooks.
