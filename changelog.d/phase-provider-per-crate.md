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
- The Endpoint resource driver now lives in its own `d2b-provider-endpoint`
  crate together with its spec decoder, its effect port, and the driver
  declaration the resource plane registers the type by. The daemon keeps the
  production effect implementation behind the port, so the driver depends on
  no provider crate, and the registry serves the type's decoder and factory
  from the declaration instead of a daemon-side table. Operator-visible
  behavior is unchanged: the same endpoint shapes are admitted, and the same
  validate, recover, reconcile, finalize, and delete verbs run.
- The Network, USB, and security-key families left the daemon's shared
  provider driver. Network now lives in `d2b-provider-network-local` beside
  the reconciler it drives, the USB Service/Binding types in
  `d2b-provider-device-usbip`, the security-key Service/Binding types in
  `d2b-provider-device-security-key`, and the `Device` type - one
  ResourceType served by four hardware Providers - in the new
  `d2b-provider-device` crate. Each crate declares its own rows, children,
  dependency references, and effect port; the daemon keeps the production
  effects behind those ports and registers every type through its declaration.
  The shared driver flow (row resolution, child ensures, owned-child
  retirement, status projection) lives in `d2b-provider-toolkit`, so no two
  families can diverge on it. Operator-visible behavior is unchanged: the same
  rows, Provider identities, controller references, repair cadences, and
  teardown ordering run, and `packages/d2bd/src/shared_provider_driver.rs` is
  gone.