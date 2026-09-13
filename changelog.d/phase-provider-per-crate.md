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
- The interaction family is now six per-type crates -
  `d2b-provider-wayland-policy`, `d2b-provider-wayland-session`,
  `d2b-provider-audio-service`, `d2b-provider-audio-binding`,
  `d2b-provider-shell-pool`, and `d2b-provider-shell-session` - one per
  resource type. Each owns its type's driver, spec decoder, factory, row
  vocabulary, and driver declaration; the six descriptors register through the
  registry, so the daemon's `interaction_driver` module, its decoder loop, and
  its type and provider literals are gone. The shared driver engine (the
  reconcile, recover, finalize, and delete verbs, the spec-envelope decode, the
  manager-child plumbing, and the effect port) lives in the family's root
  crate, the daemon keeps the production effects, and the display supervisor's
  and audio Provider's child intents reach the session and binding crates
  through ports the daemon implements. Operator-visible behavior is unchanged:
  the same six types are served, the same children are ensured in the same
  order, and the same teardown ordering runs.