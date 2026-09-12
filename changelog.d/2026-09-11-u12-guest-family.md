### Changed

- Converted the `Guest` resource family to the v3 plane. A new
  `GuestDriver`/`GuestDriverFactory` (`packages/d2bd/src/guest_driver.rs`)
  serves the four runtime Providers - cloud-hypervisor, qemu-media, azure
  container apps, azure virtual machine - as one `Guest` driver over the
  dyn-erased `GuestDriverEffects` port
  (`packages/d2bd/src/guest_effects.rs`: the CH real path plus the preserved
  framework QEMU/ACA/AzureVM controllers). Validate checks the spec against
  the owning Provider row; reconcile ensures the kind's desired children
  through the manager child API (QEMU: Volume + Process; ACA: Endpoint; CH
  children stay committed by the CH controller session), retires owned
  children the derived set no longer names endpoint-first/process-last, runs
  the provider effect, publishes in-memory status (R11) and requeues while
  not converged; delete runs the provider stage before child retirement.
  `Guest` joins `V3_CONVERTED_RESOURCE_TYPES` (31 -> 32) and is served only
  by the manager plane.
- The Cloud Hypervisor controller's layered Guest status now reaches the
  public view through the in-memory projection channel
  (`ResourceContext::set_status_projection` /
  `ResourceView.status_projection`): `UpdateStatus` captures the provider
  status into the sink instead of writing a durable row, and the
  finalizer-acknowledgement arms are no-ops because the manager's
  deleting-row hold replaces the provider finalizer (F3). The converted
  child arms of the CH session refuse closed.
- `resolve_committed_guest_session_target` falls back to the endpoint row's
  `metadata.generation` for the session target, because a manager-owned
  Endpoint row carries no durable status to stamp an endpoint generation.

### Removed

- Deleted `packages/d2bd/src/resource_runtime/guest_provider_runtime.rs`
  and `packages/d2bd/src/resource_runtime/shared_provider_runtime.rs`, and
  removed the U6 shared-Runner wave from `resource_runtime.rs` and
  `composition.rs` (registrations, runner tasks, readiness gate, shutdown
  drain, rebind hooks).
- Trimmed `binding_child_resource_runtime.rs` to its two live readers
  (`binding_readiness_current`, `parsed_binding_spec`); the guest-child
  reconciler machinery is deleted with the family.
