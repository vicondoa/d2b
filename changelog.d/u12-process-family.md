### Changed

- Retired the legacy host Process/EphemeralProcess runner: the old-plane
  `ProcessResourceReconciler` startup, its task/generation/failure fields,
  its readiness and shutdown plumbing, and the redb-backed process identity
  caches/loaders it seeded are gone. Converted `Process` rows are served
  exclusively by the new plane's `ProcessDriver`; the host in-daemon writers
  and unconverted `EphemeralProcess` rows keep their old-plane semantics.
- Split the old runner entry point: `reconcile_process_resources` becomes
  `reconcile_controller_sessions`, keeping the external-controller wake
  registration, establishment loop, and resource fences on the old plane
  (controller sessions stay there until the core-controller conversion), with
  the composition startup call retargeted and the display-host-proxy /
  cloud-hypervisor lifecycle launch nudges removed.
- Bound the catalog-bound `Guest` setup descriptor digest into the production
  Process driver effects for guest-owned rows. The private guest VMM intent
  lookup refuses a ticket without it
  (`provider-ticket:guest-descriptor-unbound`), so a controller-minted
  `Process/<guest>-vmm` row could not launch end to end; the digest is the
  same value the old runner snapshotted from the bundle's loaded Guest setup
  descriptors.
- Deleted the host-only machinery in `process_resource_runtime.rs`: the
  `ProcessResourceRuntime::new` host constructor, the redb identity-cache
  seeding and loader setters, the target-scope/lifecycle/descriptor setters
  only the host runner used, and `controller_provider_refs`.
