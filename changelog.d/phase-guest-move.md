### Changed

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
