### Removed

- Removed the Azure VM controller's mutable-update, adoption, and enrollment
  surface, which had no in-tree production caller: `AzureVmUpdate`,
  `AzureVmController::update`, `AzureVmController::adopt`,
  `AzureVmController::complete_enrollment`, `AzureVmController::status` with
  the `AzureVmStatus` projection, and `AzureVmController::controller_execution_ref`.
  The controller keeps exactly the entry points the framework Guest adapter
  drives: `reconcile`, `poll_operation` with `recovery_state`,
  `finalize`, and `finalizer_installed`.
- Removed the state that only existed to serve that surface: the
  `Reconfiguring` phase, the `pendingUpdate` field of the serialized
  `AzureVmRecoveryState` record, the pending-update field of the controller,
  and the private `validate_update`/`apply_update` helpers. The recovery
  record no longer accepts a reconfiguration phase, and the controller no
  longer computes an identity digest that no consumer could read.
