### Changed

- The controller family's nine resource types now have one driver crate each:
  `d2b-provider-zone`, `d2b-provider-zone-link`, `d2b-provider-provider`,
  `d2b-provider-role`, `d2b-provider-role-binding`, `d2b-provider-quota`,
  `d2b-provider-emergency-policy`, `d2b-provider-resource-export`, and
  `d2b-provider-resource-import`, plus the policy-type crates
  `d2b-provider-command`, `d2b-provider-operation`, and
  `d2b-provider-seccomp-profile`. Each crate owns its type's driver, spec
  decoder, and driver declaration; the daemon registers all twelve through the
  registry and no longer holds a core-family driver module. The twelve crates
  keep exactly the behavior the fixed Core process had: eight types converge as
  metadata, `Provider` re-observes its owned controller `Process` and state
  `Volume` rows and republishes its phase, and every type drains owned children
  before it retires. Failure kinds, phases, and teardown ordering are
  unchanged.
- The three policy types (`Command`, `Operation`, `SeccompProfile`) are
  declared as standard, converted resource types with their drivers registered,
  so their descriptors are reachable and the plane's driver-registry coverage
  fence closes over them. Their rows still commit with the committed policy
  rows; this change adds no seed work.
- The resource-domain modules left the controller-session library with the
  types they belong to: the Zone status projection moved to
  `d2b-provider-zone`, the zone-link enrollment state machine and its cursor
  ownership to `d2b-provider-zone-link`, the positive authorization decision
  cache to `d2b-provider-role`, and the provider lifecycle policy to
  `d2b-provider-provider`. `d2b-core-controller` keeps only resource-agnostic
  machinery - the assignment transport, the coordinator, the migration
  receipts, the fixed handler catalog, the owner-child reconciler, and the
  Host-global authority index with its durable operation adapter.
- The fixed provider identities (`Provider/system-core`,
  `Provider/system-minijail`) are taken from the crates that declare them
  (`d2b-provider-system-core`, `d2b-provider-process-minijail`) instead of
  being restated by the driver.
