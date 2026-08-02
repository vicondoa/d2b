# device-tpm integration fixtures

The Provider's heavier scenarios are intentionally kept beside the Provider:

- `provision_and_reboot/` covers the prepare, flush, swtpm, and Guest reboot
  lifecycle.
- `tamper_marker_survives/` verifies marker identity survives a Provider
  restart and that a missing marker fails closed.
- `finalizer_no_delete/` verifies Device finalization never deletes the TPM
  Volume.

These fixtures require a Host/Guest or container lane and are not part of the
hermetic Cargo tests.

