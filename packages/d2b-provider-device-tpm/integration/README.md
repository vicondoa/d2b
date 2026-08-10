# `device-tpm` integration fixtures

`provider_lifecycle.rs` declares the `host-integration` target. The scenarios
are intentionally kept beside the Provider and are invoked through
`make test-host-integration`, not as standalone scripts.

| Fixture | Required end-to-end assertion |
| --- | --- |
| `provision_and_reboot/` | state preparation, mandatory flush, swtpm start, Guest boot, and reboot adoption |
| `tamper_marker_survives/` | marker identity survives restart and a missing marker fails closed |
| `finalizer_no_delete/` | finalization stops the worker and retains the TPM Volume |

The fixtures require the existing Core effect adapter and Host/Guest harness.
They do not use an operator TPM. The hermetic Cargo tests in `tests/` remain
the fast proof of ordering, token redaction, tamper failure, and finalizer
state preservation.
