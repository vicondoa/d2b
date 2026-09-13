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
- The Volume and VolumeBinding drivers now live in their own
  `d2b-provider-volume` and `d2b-provider-volume-binding` crates, together
  with their spec decoders, their effect ports, and the driver declarations
  the resource plane registers the types by. The daemon keeps the production
  effect implementations behind those ports - the volume-local layout effect
  and its durable probe, and the binding serving socket, its removal, and the
  guest-mount observation - so the family crates carry no host state. The
  registry serves each type's decoder and factory from its declaration
  instead of a hand-built provider and decoder table, and the
  `volume_driver`, `binding_driver`, and `binding_child_resource_runtime`
  daemon modules are gone.
- The VolumeBinding declaration licenses the two children the driver mints:
  the worker `Process` served by `Provider/system-minijail` and the `Endpoint`
  served by `Provider/volume-virtiofs`, each with its creation rank. The
  Volume declaration licenses the `VolumeBinding` child its admitted
  attachments derive. Both family crates take those provider references from
  the provider crates that own them, and the binding driver's own child
  retirement order is derived from the declaration's ranks rather than from a
  second table.
- The two binding row readers (`binding_readiness_current`,
  `parsed_binding_spec`) move to the binding crate, and the daemon reads
  stored binding rows through them. `d2b-provider-volume-virtiofs` exports
  its canonical `PROVIDER_REF` so the declaring crates stop respelling it.
  Operator-visible behavior is unchanged: the same volume and binding shapes
  are admitted, the same validate, recover, reconcile, finalize, and delete
  verbs run, and the worker/endpoint teardown order is preserved.
- The Credential resource driver now lives in its own `d2b-provider-credential`
  crate together with its spec decoder, its effect port, the session and
  revocation vocabulary its teardown binds, and the driver declaration the
  resource plane registers the type by. The three Credential Providers stay
  the separate realizer crates they already are; the daemon keeps the
  production effect implementation - the Provider reads, the live session
  adapter, and the handoff registry - behind the port. Operator-visible
  behavior is unchanged: the same three Providers are admitted, the same
  per-Provider scope checks apply, the managed-identity agent Process child is
  still minted through the manager before it is spawned, and a delete still
  revokes the lease before anything owned is marked deleting.
- The managed-identity agent's Process child is now a declared `ChildCreation`
  on the Credential declaration, pinned to the minijail Process Provider's own
  exported reference rather than a daemon-side literal, and the Credential
  family's `credential_driver` module is gone from the daemon.
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
- The Host and User bootstrap drivers now live in their own
  `d2b-provider-host` and `d2b-provider-user` crates, each with the type's
  spec decoder, its effect port, and the driver declaration the resource plane
  registers the type by; `d2b-provider-system-core` keeps the Host/User
  reconciler realizers the daemon's effect implementations drive. The daemon
  registers both types through their declarations, so the daemon-side
  `system_core_driver` module and the per-type decoder table entries that only
  existed for these two types are gone. Operator-visible behavior is
  unchanged: the same Host Provider fence, the same bounded probe with its
  degraded fallback, the same local User discovery, and the same failure
  kinds.
- The Activation driver and the telemetry pair now live in their own provider
  crates: `d2b-provider-activation-nixos` owns the `NixosGeneration` driver,
  its spec decoder, its factory, and the declaration the plane registers the
  type by, and `d2b-provider-telemetry-service` and
  `d2b-provider-telemetry-binding` own the two telemetry types with their
  decoders and declarations. The daemon keeps the production activation
  effects behind the port the driver declares and registers all three types
  through their declarations, so the `activation_driver` and
  `semantic_binding_resource_runtime` modules and their decoder table entries
  are gone. Operator-visible behavior is unchanged: the same three types are
  admitted, the preserved `ApplyHostGenerationHandoff` dispatch and its closed
  result mapping run unchanged, and the telemetry pair keeps its verbs,
  execution domains, and provider-declared child creations.
- The Guest family now lives in its own `d2b-provider-guest` crate: the
  `Guest` driver over the four runtime Providers, its spec decoder and driver
  factory, the family's registration and child-creation declarations, and the
  Guest-side target-control service and host-side channel. The daemon
  registers the family through its descriptor and keeps only the production
  effect implementation behind the port the crate declares, so
  `guest_driver.rs` is gone. Operator-visible behavior is unchanged: the same
  four Providers are admitted, the Cloud Hypervisor children stay
  controller-owned, and validate, recover, reconcile, finalize, and delete run
  the same per-kind verbs.
- `d2b-provider-runtime-cloud-hypervisor`, `d2b-provider-runtime-qemu-media`,
  `d2b-provider-runtime-azure-container-apps`, and
  `d2b-provider-runtime-azure-virtual-machine` are renamed to
  `d2b-provider-guest-cloud-hypervisor`, `d2b-provider-guest-qemu-media`,
  `d2b-provider-guest-azure-container-apps`, and
  `d2b-provider-guest-azure-virtual-machine`, beside the family crate they
  realize. All four keep their provider identities
  (`Provider/runtime-cloud-hypervisor`, `Provider/runtime-qemu-media`,
  `Provider/runtime-azure-container-apps`,
  `Provider/runtime-azure-virtual-machine`), their packaging dossiers, and
  their exported names, and every Cargo, Bazel, Nix, copied-Guest, and
  packaging-matrix reference follows the new names.
- The shared-driver exemption ratchet in `check-provider-crate-layout` is
  empty: no module in a shared crate declares a resource driver, so the list
  retires its last entry with the Guest move and a driver outside a provider
  crate fails with its module path instead of an exemption.
