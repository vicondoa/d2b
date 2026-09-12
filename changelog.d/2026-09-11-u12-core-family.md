### Changed

- Converted the nine fixed Core controller-family resource types (`Zone`,
  `ZoneLink`, `Provider`, `Role`, `RoleBinding`, `Quota`,
  `EmergencyPolicy`, `ResourceExport`, `ResourceImport`) to the v3 plane:
  a new `CoreDriver`/`CoreResourceDriverFactory`
  (`packages/d2bd/src/core_driver.rs`) serves them through
  `CoreDriverEffects`. The eight metadata-only types converge without
  effects; `Provider` observes its manager-served owned `Process`/`Volume`
  rows plus the fixed Host/Zone rows and runs the preserved
  `provider_observation` / `ProviderHandler::plan_observed` logic, publishes
  its typed status in memory (R11), and finalizes children-first with the
  controller-drain gate before converging. The nine types join
  `V3_CONVERTED_RESOURCE_TYPES` and are served only by the manager plane.

### Removed

- Deleted the Core reconciler surface from `d2b-core-controller`:
  `CoreResourceReconciler` (definition, `ResourceReconciler` impl and
  tests), `core_controller_descriptors` /
  `CoreControllerDescriptorError`, `CoreResourceControllerRegistration`,
  `CORE_RESOURCE_CONTROLLER_REGISTRATIONS`, and
  `CORE_PROVIDER_API_BINDING_FINALIZER`; the crate's re-exports are trimmed
  to the surviving contract pieces (`fixed_system_core_handlers_ready`
  stays).
