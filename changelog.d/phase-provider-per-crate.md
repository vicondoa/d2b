### Changed

- The Process family now lives in its own `d2b-provider-process` crate: the
  driver for `Process` and `EphemeralProcess`, the family's declarations, the
  spec decoder and driver factory the registry serves, the one canonical
  launch-identity resolver, and the process launch and supervision primitives
  the family exchanges. The daemon registers the family through its
  descriptors and keeps only the production effect implementation behind the
  port the crate declares. Its `process_driver` module is gone.
- `d2b-provider-system-minijail` and `d2b-provider-system-systemd` are renamed
  to `d2b-provider-process-minijail` and `d2b-provider-process-systemd`, beside
  the family crate they realize. Both keep their provider identities
  (`Provider/system-minijail`, `Provider/system-systemd`) and their exported
  names, and every Cargo, Bazel, Nix, copied-Guest, and packaging-matrix
  reference follows the new names.
- `check-provider-crate-layout` now refuses a resource driver declared in a
  shared crate: the still-un-migrated families are an explicit, shrinking
  exemption list in the policy crate, and a new driver outside a provider
  crate fails with its module path. The integration-scenario ratchet records
  the crates whose integration surface is still a scaffold.
- The Process provider effect port now lives in the `d2b-process` crate
  together with the Process-family spec, row-identity, and typed worker
  launch-parameter types it exchanges; the daemon keeps the production
  effect implementation and the driver consumes the port from its new home.
  Device-worker launch parameters cross that boundary as canonical JSON
  instead of a provider-crate Rust type, so no family crate depends on a
  realizer crate. The `system-minijail` and `system-systemd` crates export
  their canonical `Provider/...` references, and the Process driver consumes
  them instead of restating the provider strings locally.
