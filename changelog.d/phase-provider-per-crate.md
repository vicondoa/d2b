### Changed

- The Process provider effect port now lives in the `d2b-process` crate
  together with the Process-family spec, row-identity, and typed worker
  launch-parameter types it exchanges; the daemon keeps the production
  effect implementation and the driver consumes the port from its new home.
  Device-worker launch parameters cross that boundary as canonical JSON
  instead of a provider-crate Rust type, so no family crate depends on a
  realizer crate. The `system-minijail` and `system-systemd` crates export
  their canonical `Provider/...` references, and the Process driver consumes
  them instead of restating the provider strings locally.
