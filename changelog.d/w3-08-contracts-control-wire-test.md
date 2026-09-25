### Changed

- Added a wire-shape and round-trip test for the workload operation family (`WorkloadOp` List/Status/LauncherExec and `WorkloadOpResponse`) in the contracts-control crate, pinning the `kind`/`op` tags, camelCase field renames, and canonical target encoding so future DTO edits cannot silently break version-3 peers.