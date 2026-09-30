### Added

- `volume-virtiofs` now derives a serving worker's source and socket from
  the admitted `VolumeBinding` instead of composing them from a launch
  argument. `StoredBinding::serving_source` resolves the named view to a
  `ServingSource` locator (a declared storage row, or the broker-managed
  store-view farm), and `StoredBinding::serving_socket_path` derives the
  private listening socket from the binding's own opaque socket identity
  under the broker's runtime root. The binding-derived `ServingWorkerLaunch`
  carries both to the effect port, so an adapter receives no argv, no view
  root, and no Guest or Device row and cannot re-point the launch it was
  given.
- The provider's serving component declares the
  `namespace-first-service-source` presentation capability and its two setup
  restrictions (`steady-state-mount-namespace`, `zero-host-capability`) on
  the plan itself, instead of inheriting a setup mode from the old
  `virtiofsd-worker` launch role or its seccomp label.
- `d2b-broker` gained a binding-scoped store-view export:
  `StoreViewExportBinding` names the consumer, the named view, and the single
  generation that consumer is admitted for, refuses a writable export of a
  shared closure farm outright, and refuses every mutation that would reach
  a shared content-store inode through the farm's hardlinks
  (`FarmMutation::REFUSED_SET`). `build_read_only_store_view` materialises
  one admitted export, and `run_store_sync_for_export` publishes one under
  the same generation fence, refusing before the lock is taken and before
  anything is created.

### Changed

- A binding's readiness now separates source PREPARATION from the
  consumer's mount COMPLETION. The fenced `VolumeBinding` status projection
  reports the pre-start condition, so a Guest with a virtiofs Volume boots
  from a `Prepared` export and reports mount completion afterwards instead of
  the two conditions waiting on each other. `BindingPhase::Prepared` is the
  new pre-boot steady state, `Degraded` now means a running consumer reports
  no mount, and `observe_guest_mount` returns a tri-state
  `MountObservation` so "the consumer has not started" is distinguishable
  from "the consumer is running and the mount is gone".
- The daemon's serving-worker launch binds its private socket directly in the
  broker's own runtime root. It no longer reads a `path:vm-run:<guest>`
  storage row for a per-Guest directory mode and no longer creates or posts
  a per-Guest socket directory, so a Guest with no Device children derives
  and prepares its export exactly like any other Guest. The
  `declared_vm_run_dir_mode` resolver, the socket-directory realization, and
  their two owner tests are deleted.
